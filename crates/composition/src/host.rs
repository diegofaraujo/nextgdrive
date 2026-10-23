//! Background sync as a long-running service: the loop shared by the tray
//! app and `nextgdrive daemon`, controllable in-process and over D-Bus.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use nextgdrive_adapter_dbus::{
    HostControl, HostStatus, STATE_IDLE, STATE_PAUSED, STATE_SYNCING, ServeError,
};
use nextgdrive_application::AppError;
use std::collections::HashSet;

use nextgdrive_application::background::{SyncEvent, TickOptions};
use nextgdrive_application::ports::AccountRepository;
use nextgdrive_domain::{AccountId, Email};
use nextgdrive_i18n::{FluentValue, Localizer};
use tokio::sync::{mpsc, watch};

use crate::Services;

#[derive(Debug, Clone)]
enum Command {
    SyncNow,
    Pause,
    Resume,
    /// Trash the many local deletions of this account (by email).
    ConfirmDeletions(String),
}

/// Translated line describing something background sync did, for logs.
pub type LogSink = Box<dyn Fn(String) + Send + Sync>;

/// Handle to the running background sync. Dropping it stops the sync.
pub struct SyncHost {
    control: Arc<Control>,
    task: tokio::task::JoinHandle<()>,
    _bus: zbus::Connection,
}

/// Shared with the D-Bus service and the loop.
struct Control {
    commands: mpsc::UnboundedSender<Command>,
    status: watch::Receiver<HostStatus>,
}

impl HostControl for Control {
    fn status(&self) -> HostStatus {
        self.status.borrow().clone()
    }

    fn sync_now(&self) {
        let _ = self.commands.send(Command::SyncNow);
    }

    fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    fn confirm_deletions(&self, email: &str) {
        let _ = self
            .commands
            .send(Command::ConfirmDeletions(email.to_owned()));
    }
}

#[derive(Debug)]
pub enum StartError {
    /// Another process (tray app or daemon) already runs background sync.
    AlreadyRunning,
    Bus(String),
}

impl SyncHost {
    /// Starts background sync on the current Tokio runtime and publishes it on
    /// D-Bus. Only one process per session may run it.
    pub async fn start(
        services: Arc<Services>,
        i18n: Localizer,
        log: LogSink,
    ) -> Result<Self, StartError> {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (status_tx, status) = watch::channel(HostStatus {
            state: STATE_IDLE.into(),
            ..HostStatus::default()
        });
        let control = Arc::new(Control { commands, status });
        let bus = nextgdrive_adapter_dbus::serve(control.clone())
            .await
            .map_err(|e| match e {
                ServeError::AlreadyRunning => StartError::AlreadyRunning,
                ServeError::Bus(e) => StartError::Bus(e.to_string()),
            })?;
        let task = tokio::spawn(run(services, i18n, log, receiver, status_tx));
        Ok(Self {
            control,
            task,
            _bus: bus,
        })
    }

    pub fn status(&self) -> watch::Receiver<HostStatus> {
        self.control.status.clone()
    }

    pub fn sync_now(&self) {
        self.control.sync_now();
    }

    pub fn pause(&self) {
        self.control.pause();
    }

    pub fn resume(&self) {
        self.control.resume();
    }

    pub fn confirm_deletions(&self, email: &str) {
        self.control.confirm_deletions(email);
    }
}

impl Drop for SyncHost {
    /// Cancelling mid-sync is safe: partial downloads are discarded and the
    /// account lock is released.
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn unix_now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    i64::try_from(secs).unwrap_or(i64::MAX)
}

async fn run(
    services: Arc<Services>,
    i18n: Localizer,
    log: LogSink,
    mut commands: mpsc::UnboundedReceiver<Command>,
    status: watch::Sender<HostStatus>,
) {
    let background = services.background_sync();
    let mut paused = false;
    let mut force = false;
    let mut confirmed: HashSet<AccountId> = HashSet::new();
    // Local changes, reported by the file watcher.
    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<()>();
    let mut watched: Option<(
        std::path::PathBuf,
        nextgdrive_adapter_localfs::ChangeWatcher,
    )> = None;
    let mut local_changes = false;
    // Confirming applies to the next sync only.
    let account_id = |email: &str| -> Option<AccountId> {
        let email = Email::parse(email).ok()?;
        Some(services.local.accounts.find_by_email(&email).ok()??.id)
    };
    loop {
        // Watch the sync folder, again whenever it changes (or appears).
        if let Ok(settings) = services.local.get_settings().execute()
            && watched.as_ref().map(|(root, _)| root) != Some(&settings.sync_root)
        {
            let tx = local_tx.clone();
            watched = nextgdrive_adapter_localfs::watch(&settings.sync_root, move || {
                let _ = tx.send(());
            })
            .ok()
            .map(|watcher| (settings.sync_root, watcher));
        }
        if !paused {
            let errors = Mutex::new(Vec::new());
            let finished = {
                let on_event =
                    |event: SyncEvent<'_>| on_event(&i18n, &log, &status, &errors, event);
                let options = TickOptions {
                    force: std::mem::take(&mut force),
                    conflict_label: crate::conflict_label(&i18n),
                    confirmed_deletions: std::mem::take(&mut confirmed),
                    local_changes: std::mem::take(&mut local_changes),
                };
                let tick = background.tick(&options, &on_event);
                tokio::pin!(tick);
                // Commands arriving mid-sync: pausing cancels it, the rest wait.
                loop {
                    tokio::select! {
                        result = &mut tick => break Some(result),
                        // Includes our own downloads: one extra, quiet check
                        // afterwards, so edits made during a sync are not missed.
                        Some(()) = local_rx.recv() => local_changes = true,
                        command = commands.recv() => match command {
                            None => return,
                            Some(Command::Pause) => {
                                paused = true;
                                break None;
                            }
                            Some(Command::SyncNow) => force = true,
                            Some(Command::ConfirmDeletions(email)) => confirmed.extend(account_id(&email)),
                            Some(Command::Resume) => {}
                        },
                    }
                }
            };
            let mut errors = errors.into_inner().unwrap_or_default();
            if let Some(Err(e)) = &finished {
                errors.push((String::new(), i18n.app_error(e)));
            }
            status.send_modify(|s| {
                s.state = if paused { STATE_PAUSED } else { STATE_IDLE }.into();
                s.account.clear();
                if finished.is_some() {
                    s.last_check = unix_now();
                    s.errors = errors;
                }
            });
        }

        let interval = services
            .local
            .get_settings()
            .execute()
            .map(|s| s.poll_interval_secs)
            .unwrap_or(nextgdrive_domain::Settings::DEFAULT_POLL_INTERVAL_SECS);
        // A forced sync requested during the last one runs right away.
        if (force || local_changes) && !paused {
            settle(&mut local_rx).await;
            continue;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(interval)), if !paused => {}
            Some(()) = local_rx.recv() => {
                settle(&mut local_rx).await;
                local_changes = true;
            }
            command = commands.recv() => match command {
                None => return,
                Some(Command::SyncNow) => {
                    force = true;
                    if paused {
                        // Syncing now implies resuming.
                        paused = false;
                    }
                }
                Some(Command::Pause) => {
                    paused = true;
                    status.send_modify(|s| s.state = STATE_PAUSED.into());
                }
                Some(Command::Resume) => {
                    paused = false;
                    force = true;
                }
                Some(Command::ConfirmDeletions(email)) => {
                    confirmed.extend(account_id(&email));
                    paused = false;
                }
            },
        }
    }
}

/// Waits until local changes stop for a moment, so that saving a file (often
/// several writes and renames) or copying a folder causes a single sync.
async fn settle(events: &mut mpsc::UnboundedReceiver<()>) {
    const QUIET: Duration = Duration::from_secs(2);
    while let Ok(Some(())) = tokio::time::timeout(QUIET, events.recv()).await {}
}

fn on_event(
    i18n: &Localizer,
    log: &LogSink,
    status: &watch::Sender<HostStatus>,
    errors: &Mutex<Vec<(String, String)>>,
    event: SyncEvent<'_>,
) {
    let push_error = |email: &str, e: &AppError| {
        if let Ok(mut errors) = errors.lock() {
            errors.push((email.to_owned(), i18n.app_error(e)));
        }
    };
    match event {
        SyncEvent::Started(account) => {
            status.send_modify(|s| {
                s.state = STATE_SYNCING.into();
                s.account = account.email.to_string();
                (s.files_done, s.files_total, s.bytes_done, s.bytes_total) = (0, 0, 0, 0);
            });
            log(i18n.tr_args(
                "sync-started",
                [("email", account.email.to_string().into())],
            ));
        }
        SyncEvent::Progress(_, p) => status.send_modify(|s| {
            s.files_done = p.files_done as u64;
            s.files_total = p.files_total as u64;
            s.bytes_done = p.bytes_done;
            s.bytes_total = p.bytes_total;
        }),
        SyncEvent::Finished(account, Ok(report)) => {
            log(format!("{}: {}", account.email, i18n.sync_summary(report)));
            for copy in &report.conflict_copies {
                log(i18n.tr_args(
                    "sync-conflict-copy",
                    [("path", copy.display().to_string().into())],
                ));
            }
            for conflict in &report.conflicts {
                log(i18n.conflict_line(conflict));
            }
        }
        // Someone else (e.g. a one-off `nextgdrive sync run`) is on it.
        SyncEvent::Finished(_, Err(AppError::SyncInProgress(_))) => {}
        SyncEvent::Finished(account, Err(e)) | SyncEvent::CheckFailed(account, e) => {
            push_error(account.email.as_str(), e);
            log(format!("{}: {}", account.email, i18n.app_error(e)));
        }
        SyncEvent::UpToDate(_) => {}
    }
}

/// One-line, translated summary of a status, as shown in the tray and by
/// `nextgdrive status`.
pub fn describe(i18n: &Localizer, status: &HostStatus) -> String {
    let line = match status.state.as_str() {
        STATE_PAUSED => i18n.tr("status-paused"),
        STATE_SYNCING if status.files_total == 0 => i18n.tr_args(
            "status-checking",
            [("email", status.account.clone().into())],
        ),
        STATE_SYNCING => i18n.tr_args(
            "status-syncing",
            [
                ("email", status.account.clone().into()),
                ("done", FluentValue::from(status.files_done)),
                ("total", FluentValue::from(status.files_total)),
            ],
        ),
        _ if status.last_check == 0 => i18n.tr("status-starting"),
        _ => {
            let minutes = (unix_now() - status.last_check).max(0) / 60;
            let ago = i18n.tr_args(
                "status-checked-ago",
                [("minutes", FluentValue::from(minutes))],
            );
            format!("{} · {ago}", i18n.tr("status-up-to-date"))
        }
    };
    match status.errors.first() {
        Some((email, message)) if !email.is_empty() => {
            let problem = i18n.tr_args(
                "status-problem",
                [
                    ("email", email.clone().into()),
                    ("message", message.clone().into()),
                ],
            );
            format!("{line}\n{problem}")
        }
        Some((_, message)) => format!("{line}\n{message}"),
        None => line,
    }
}

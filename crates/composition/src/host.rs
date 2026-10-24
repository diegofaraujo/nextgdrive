//! Background sync as a long-running service: the loop shared by the tray
//! app and `nextgdrive daemon`, controllable in-process and over D-Bus.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use nextgdrive_adapter_dbus::{
    HostControl, HostStatus, STATE_IDLE, STATE_PAUSED, STATE_SYNCING, ServeError,
};
use nextgdrive_application::AppError;
use std::collections::HashSet;

use nextgdrive_application::background::{SyncEvent, TickOptions};
use nextgdrive_application::ports::AccountRepository;
use nextgdrive_domain::{Account, AccountId, Email, Settings, SyncMode};
use nextgdrive_i18n::{FluentValue, Localizer};
use tokio::sync::{mpsc, watch};

use crate::Services;

#[derive(Debug)]
enum Command {
    SyncNow,
    Pause,
    Resume,
    /// Trash the many local deletions of this account (by email).
    ConfirmDeletions(String),
    /// Unmount streamed folders and stop; answered once done.
    Shutdown(tokio::sync::oneshot::Sender<()>),
}

/// Translated line describing something background sync did, for logs.
pub type LogSink = Box<dyn Fn(String) + Send + Sync>;

/// Receives what deserves the user's attention, for desktop notifications.
pub type NoticeSink = Box<dyn Fn(Notice) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Files changed on both sides were kept as conflict copies.
    ConflictCopies,
    /// Items were left alone (read-only, cannot be deleted…).
    NeedsAttention,
    /// Many local deletions wait for the user's decision.
    ConfirmDeletions,
    /// Syncing an account failed.
    Problem,
}

/// Something to tell the user, already translated.
#[derive(Debug, Clone)]
pub struct Notice {
    pub kind: NoticeKind,
    pub account: String,
    pub title: String,
    pub body: String,
}

/// Where background sync reports to. Each problem is notified once, until
/// the account syncs fine again.
struct Reporter {
    log: LogSink,
    notify: NoticeSink,
    last_problem: Mutex<std::collections::HashMap<String, String>>,
}

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
        notify: NoticeSink,
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
        let reporter = Reporter {
            log,
            notify,
            last_problem: Mutex::default(),
        };
        let task = tokio::spawn(run(services, i18n, Arc::new(reporter), receiver, status_tx));
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

    /// Unmounts streamed folders and stops background sync. Call it before
    /// the process exits.
    pub async fn shutdown(&self) {
        let (done, wait) = tokio::sync::oneshot::channel();
        if self.control.commands.send(Command::Shutdown(done)).is_ok() {
            let _ = wait.await;
        }
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
    reporter: Arc<Reporter>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    status: watch::Sender<HostStatus>,
) {
    let background = services.background_sync();
    let mut paused = false;
    let mut force = false;
    let mut confirmed: HashSet<AccountId> = HashSet::new();
    // Local changes, reported by the file watcher.
    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<()>();
    // One watcher per mirrored account folder: never the sync folder as a
    // whole, which holds streamed mounts a recursive watch would walk.
    let mut watched: HashMap<PathBuf, nextgdrive_adapter_localfs::ChangeWatcher> = HashMap::new();
    let mut mounts = Mounts::default();
    let mut local_changes = false;
    // Confirming applies to the next sync only.
    let account_id = |email: &str| -> Option<AccountId> {
        let email = Email::parse(email).ok()?;
        Some(services.local.accounts.find_by_email(&email).ok()??.id)
    };
    loop {
        if let (Ok(settings), Ok(accounts)) = (
            services.local.get_settings().execute(),
            services.local.list_accounts().execute(),
        ) {
            // Mounts first: a folder switched back to mirroring must be
            // unmounted before it is watched, or the recursive watch walks the
            // whole streamed Drive (and, answered on this runtime, can hang it).
            mounts.reconcile(&services, &settings, &accounts, &i18n, &reporter);
            let mirrored: Vec<PathBuf> = accounts
                .iter()
                .filter(|a| settings.mode(&a.id) == SyncMode::Mirror)
                .map(|a| settings.account_dir(a))
                .filter(|dir| !mounts.is_mounted(dir) && dir.is_dir())
                .collect();
            watched.retain(|dir, _| mirrored.contains(dir));
            for dir in mirrored {
                if let std::collections::hash_map::Entry::Vacant(slot) = watched.entry(dir) {
                    let tx = local_tx.clone();
                    let watch = nextgdrive_adapter_localfs::watch(slot.key(), move || {
                        let _ = tx.send(());
                    });
                    if let Ok(watcher) = watch {
                        slot.insert(watcher);
                    }
                }
            }
        }
        if !paused {
            let errors = Mutex::new(Vec::new());
            let finished = {
                let on_event =
                    |event: SyncEvent<'_>| on_event(&i18n, &reporter, &status, &errors, event);
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
                            Some(Command::Shutdown(done)) => {
                                mounts.unmount_all().await;
                                let _ = done.send(());
                                return;
                            }
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
                Some(Command::Shutdown(done)) => {
                    mounts.unmount_all().await;
                    let _ = done.send(());
                    return;
                }
                Some(Command::ConfirmDeletions(email)) => {
                    confirmed.extend(account_id(&email));
                    paused = false;
                }
            },
        }
    }
}

/// Streamed accounts mounted by this host, keyed by account.
#[derive(Default)]
struct Mounts {
    mounted: HashMap<AccountId, (PathBuf, crate::stream::StreamMount)>,
    cache: Option<Arc<nextgdrive_adapter_cache::DiskBlockCache>>,
    /// Keeps offline items downloaded; one run at a time.
    offline: Option<tokio::task::JoinHandle<()>>,
}

impl Mounts {
    /// Unmounts everything, waiting until done: before the process exits,
    /// or the folders are left as dead mounts ("not connected").
    /// Pins the files of every streamed account's offline items and
    /// downloads what is missing, in the background.
    fn refresh_offline(
        &mut self,
        services: &Arc<Services>,
        accounts: Vec<Account>,
        reporter: &Arc<Reporter>,
        i18n: &Localizer,
    ) {
        if self
            .offline
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let preloaders: HashMap<AccountId, crate::stream::Preloader> = self
            .mounted
            .iter()
            .map(|(id, (_, mount))| (id.clone(), mount.preloader()))
            .collect();
        let (services, reporter, i18n) = (services.clone(), reporter.clone(), i18n.clone());
        self.offline = Some(tokio::spawn(async move {
            let offline = services.offline(&cache);
            let mut pinned = Vec::new();
            let mut wanted = Vec::new();
            for account in accounts {
                match offline.files(&account).await {
                    Ok(files) => {
                        pinned.extend(files.iter().map(|f| (f.id.clone(), f.version.clone())));
                        wanted.push((account, files));
                    }
                    Err(e) => (reporter.log)(format!("{}: {}", account.email, i18n.app_error(&e))),
                }
            }
            // Pinned first: what is about to be downloaded is never evicted.
            nextgdrive_application::ports::BlockCache::pin(&*cache, &pinned);
            for (account, files) in wanted {
                if let Err(e) = offline.download(&account, &files).await {
                    (reporter.log)(format!("{}: {}", account.email, i18n.app_error(&e)));
                }
                // Their folders too, so they can be browsed without internet.
                if let Some(preloader) = preloaders.get(&account.id) {
                    let settings = services.local.get_settings().execute();
                    let ids: Vec<_> = settings
                        .map(|s| s.offline_items(&account.id).cloned().collect())
                        .unwrap_or_default();
                    preloader.preload(&ids).await;
                }
            }
        }));
    }

    async fn unmount_all(&mut self) {
        if let Some(task) = self.offline.take() {
            task.abort();
        }
        let unmounts: Vec<_> = self
            .mounted
            .drain()
            .map(|(_, mount)| tokio::task::spawn_blocking(move || drop(mount)))
            .collect();
        for unmount in unmounts {
            let _ = unmount.await;
        }
    }

    fn is_mounted(&self, dir: &std::path::Path) -> bool {
        self.mounted.values().any(|(mounted, _)| mounted == dir)
    }

    /// Mounts accounts switched to streaming and unmounts the others.
    fn reconcile(
        &mut self,
        services: &Arc<Services>,
        settings: &Settings,
        accounts: &[Account],
        i18n: &Localizer,
        reporter: &Arc<Reporter>,
    ) {
        // A mount left by a crash makes the folder unusable in any mode.
        for account in accounts {
            crate::stream::clear_stale_mount(&settings.account_dir(account));
        }
        let wanted: HashMap<AccountId, (&Account, PathBuf)> = accounts
            .iter()
            .filter(|a| settings.mode(&a.id) == SyncMode::Stream)
            .map(|a| (a.id.clone(), (a, settings.account_dir(a))))
            .collect();
        // Unmounted when dropped (also remounted if the sync folder moved).
        // Unmounting runs `fusermount`: not on this runtime's threads.
        let stale: Vec<AccountId> = self
            .mounted
            .iter()
            .filter(|(id, (dir, _))| {
                wanted
                    .get(*id)
                    .is_none_or(|(_, wanted_dir)| wanted_dir != dir)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(mount) = self.mounted.remove(&id) {
                tokio::task::spawn_blocking(move || drop(mount));
            }
        }
        let streamed: Vec<Account> = wanted.values().map(|(a, _)| (*a).clone()).collect();
        for (id, (account, dir)) in wanted {
            if self.mounted.contains_key(&id) {
                continue;
            }
            let problem = |message: String| {
                reporter.problem(Notice {
                    kind: NoticeKind::Problem,
                    title: i18n.tr_args(
                        "notify-mount-failed-title",
                        [("email", account.email.to_string().into())],
                    ),
                    body: message,
                    account: account.email.to_string(),
                });
            };
            if std::fs::create_dir_all(&dir).is_err() || !crate::stream::is_empty_dir(&dir) {
                problem(i18n.tr_args(
                    "error-mount-not-empty",
                    [("path", dir.display().to_string().into())],
                ));
                continue;
            }
            let cache = match &self.cache {
                Some(cache) => cache.clone(),
                None => match crate::stream::open_cache(i18n, settings.cache_limit) {
                    Ok(cache) => self.cache.insert(cache).clone(),
                    Err(message) => return problem(message),
                },
            };
            match crate::stream::mount(
                services.clone(),
                cache,
                account.clone(),
                &dir,
                i18n.clone(),
                stream_events(reporter.clone(), i18n.clone()),
            ) {
                Ok(mount) => {
                    reporter.recovered(account.email.as_str());
                    self.mounted.insert(id, (dir, mount));
                }
                Err(message) => problem(message),
            }
        }
        if !streamed.is_empty() {
            self.refresh_offline(services, streamed, reporter, i18n);
        }
    }
}

/// Mount events, as logs and notifications.
fn stream_events(reporter: Arc<Reporter>, i18n: Localizer) -> crate::stream::EventSink {
    use crate::stream::StreamEvent;
    Box::new(move |event| match event {
        StreamEvent::Log(line) => (reporter.log)(line),
        StreamEvent::ConflictCopy { account, name } => {
            (reporter.log)(i18n.tr_args("sync-conflict-copy", [("path", name.clone().into())]));
            reporter.notice(Notice {
                kind: NoticeKind::ConflictCopies,
                title: i18n.tr_args("notify-conflict-title", [("count", FluentValue::from(1))]),
                body: name,
                account,
            });
        }
        StreamEvent::UploadFailed {
            account,
            name,
            message,
        } => {
            (reporter.log)(format!("{account}: {name}: {message}"));
            reporter.problem(Notice {
                kind: NoticeKind::Problem,
                title: i18n.tr_args("notify-upload-failed-title", [("name", name.into())]),
                body: message,
                account,
            });
        }
    })
}

/// Waits until local changes stop for a moment, so that saving a file (often
/// several writes and renames) or copying a folder causes a single sync.
async fn settle(events: &mut mpsc::UnboundedReceiver<()>) {
    const QUIET: Duration = Duration::from_secs(2);
    while let Ok(Some(())) = tokio::time::timeout(QUIET, events.recv()).await {}
}

fn on_event(
    i18n: &Localizer,
    reporter: &Reporter,
    status: &watch::Sender<HostStatus>,
    errors: &Mutex<Vec<(String, String)>>,
    event: SyncEvent<'_>,
) {
    let log = &reporter.log;
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
            let email = account.email.to_string();
            log(format!("{email}: {}", i18n.sync_summary(report)));
            for copy in &report.conflict_copies {
                log(i18n.tr_args(
                    "sync-conflict-copy",
                    [("path", copy.display().to_string().into())],
                ));
            }
            for conflict in &report.conflicts {
                log(i18n.conflict_line(conflict));
            }
            reporter.recovered(&email);
            let names = |paths: Vec<&std::path::Path>| file_list(i18n, &paths);
            if !report.conflict_copies.is_empty() {
                let paths = report
                    .conflict_copies
                    .iter()
                    .map(PathBuf::as_path)
                    .collect();
                reporter.notice(Notice {
                    kind: NoticeKind::ConflictCopies,
                    title: i18n.tr_args(
                        "notify-conflict-title",
                        [("count", FluentValue::from(report.conflict_copies.len()))],
                    ),
                    body: names(paths),
                    account: email.clone(),
                });
            }
            if !report.conflicts.is_empty() {
                let paths = report.conflicts.iter().map(|c| c.path.as_path()).collect();
                reporter.notice(Notice {
                    kind: NoticeKind::NeedsAttention,
                    title: i18n.tr_args(
                        "notify-attention-title",
                        [("count", FluentValue::from(report.conflicts.len()))],
                    ),
                    body: names(paths),
                    account: email,
                });
            }
        }
        // Someone else (e.g. a one-off `nextgdrive sync run`) is on it.
        SyncEvent::Finished(_, Err(AppError::SyncInProgress(_))) => {}
        SyncEvent::Finished(account, Err(e)) | SyncEvent::CheckFailed(account, e) => {
            let email = account.email.to_string();
            let message = i18n.app_error(e);
            if let Ok(mut errors) = errors.lock() {
                errors.push((email.clone(), message.clone()));
            }
            log(format!("{email}: {message}"));
            let kind = match e {
                AppError::MassDeletion { .. } => NoticeKind::ConfirmDeletions,
                _ => NoticeKind::Problem,
            };
            reporter.problem(Notice {
                kind,
                title: i18n.tr_args("notify-problem-title", [("email", email.clone().into())]),
                body: message,
                account: email,
            });
        }
        SyncEvent::UpToDate(account) => reporter.recovered(account.email.as_str()),
    }
}

impl Reporter {
    fn notice(&self, notice: Notice) {
        (self.notify)(notice);
    }

    /// Notifies a problem unless the same one was already notified.
    fn problem(&self, notice: Notice) {
        let Ok(mut last) = self.last_problem.lock() else {
            return;
        };
        if last.get(&notice.account) == Some(&notice.body) {
            return;
        }
        last.insert(notice.account.clone(), notice.body.clone());
        drop(last);
        self.notice(notice);
    }

    fn recovered(&self, email: &str) {
        if let Ok(mut last) = self.last_problem.lock() {
            last.remove(email);
        }
    }
}

/// "a.txt, b.txt and 3 more": file names only, to keep notifications short.
fn file_list(i18n: &Localizer, paths: &[&std::path::Path]) -> String {
    const SHOWN: usize = 3;
    let names: Vec<String> = paths
        .iter()
        .take(SHOWN)
        .map(|p| {
            p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let list = names.join(", ");
    match paths.len().saturating_sub(SHOWN) {
        0 => list,
        more => i18n.tr_args(
            "notify-and-more",
            [("list", list.into()), ("count", FluentValue::from(more))],
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reporter(sent: Arc<Mutex<Vec<String>>>) -> Reporter {
        Reporter {
            log: Box::new(|_| {}),
            notify: Box::new(move |n| sent.lock().unwrap().push(n.body)),
            last_problem: Mutex::default(),
        }
    }

    fn problem(body: &str) -> Notice {
        Notice {
            kind: NoticeKind::Problem,
            account: "ana@x.com".into(),
            title: String::new(),
            body: body.into(),
        }
    }

    #[test]
    fn each_problem_is_notified_once_until_it_recovers() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let reporter = reporter(sent.clone());

        reporter.problem(problem("offline"));
        reporter.problem(problem("offline"));
        reporter.problem(problem("token expired"));
        reporter.recovered("ana@x.com");
        reporter.problem(problem("offline"));

        assert_eq!(
            *sent.lock().unwrap(),
            ["offline", "token expired", "offline"]
        );
    }

    #[test]
    fn long_file_lists_are_shortened() {
        let i18n = Localizer::new("en-US");
        let paths: Vec<PathBuf> = ["/d/a.txt", "/d/b.txt", "/d/c.txt", "/d/e.txt", "/d/f.txt"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let paths: Vec<&std::path::Path> = paths.iter().map(PathBuf::as_path).collect();
        assert_eq!(file_list(&i18n, &paths), "a.txt, b.txt, c.txt and 2 more");
        assert_eq!(file_list(&i18n, &paths[..1]), "a.txt");
    }
}

//! Streaming mounts: an account's whole Drive as a folder, read on demand and
//! written through a staging area.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nextgdrive_adapter_cache::DiskBlockCache;
use nextgdrive_adapter_fuse::{Mount, StreamBackend};
use nextgdrive_adapter_localfs::DiskStaging;
use nextgdrive_adapter_sqlite::SqliteDriveIndex;
use nextgdrive_application::streaming::{
    Entry, IndexedDrive, NodeAttr, NodeId, StreamConfig, StreamError, StreamState, StreamingDrive,
    UpdateIndex, UploadOutcome,
};
use nextgdrive_domain::{Account, StorageQuota};
use nextgdrive_i18n::Localizer;

use crate::Services;

const QUOTA_TTL: Duration = Duration::from_secs(5 * 60);
/// How often staged files are checked for uploading: new files once quiet
/// (`StreamConfig::new_file_quiet`), failed ones after their backoff.
const UPLOAD_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Folders listed at once in the background (stale ones, and subfolders
/// listed ahead of being opened).
const LISTING_WORKERS: usize = 4;

/// Something a mount did that is worth telling, already translated.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Log(String),
    /// A file changed in Drive while being edited here; this copy kept the edit.
    ConflictCopy {
        account: String,
        name: String,
    },
    /// Uploading failed; the content is safe and will be retried.
    UploadFailed {
        account: String,
        name: String,
        message: String,
    },
}

pub type EventSink = Box<dyn Fn(StreamEvent) + Send + Sync>;

type Drive = IndexedDrive<crate::GoogleDrive, Arc<SqliteDriveIndex>>;

struct AccountStream {
    services: Arc<Services>,
    /// Drive as this mount sees it: listed from the index once it is built.
    drive: Drive,
    cache: Arc<DiskBlockCache>,
    staging: Arc<DiskStaging>,
    account: Account,
    state: StreamState,
    quota: Mutex<Option<(Instant, StorageQuota)>>,
    events: EventSink,
    i18n: Localizer,
    /// Offline items whose folders were listed in this mount already.
    preloaded: Mutex<std::collections::HashSet<nextgdrive_domain::DriveItemId>>,
    /// Wakes the task listing folders in the background.
    refresh: tokio::sync::Notify,
}

type Fs<'a> = StreamingDrive<
    'a,
    crate::GoogleAuthenticator,
    crate::KeyringCredentialStore,
    Drive,
    DiskBlockCache,
    crate::TomlSettingsRepository,
    DiskStaging,
>;

impl AccountStream {
    fn fs(&self) -> Fs<'_> {
        StreamingDrive::new(
            &self.services.tokens,
            &self.drive,
            &*self.cache,
            &self.services.local.settings,
            &*self.staging,
            &self.state,
            &self.account,
            crate::conflict_label(&self.i18n),
        )
    }

    fn log_outcome(&self, outcome: &UploadOutcome) {
        if let Some(copy) = &outcome.conflict_copy {
            (self.events)(StreamEvent::ConflictCopy {
                account: self.account.email.to_string(),
                name: copy.clone(),
            });
        }
    }

    /// Lets the background task list what browsing queued.
    fn wake_refresh(&self) {
        if self.fs().refresh_pending() {
            self.refresh.notify_one();
        }
    }

    /// Lists queued folders, `LISTING_WORKERS` at a time, until none is left.
    async fn refresh_listings(self: &Arc<Self>) {
        let mut workers = tokio::task::JoinSet::new();
        for _ in 0..LISTING_WORKERS {
            let stream = self.clone();
            workers.spawn(async move { while stream.fs().refresh_next().await {} });
        }
        while workers.join_next().await.is_some() {}
    }

    /// Brings the index up to date and forgets the listings it changed.
    /// Failures (e.g. offline) are told once, until it works again.
    async fn update_index(&self, failing: &mut bool) {
        let index = &*self.services.local.index;
        let result = UpdateIndex::new(&self.services.tokens, &self.services.drive, index)
            .execute(&self.account)
            .await;
        match result {
            Ok(update) => {
                self.state.index_changed(&update);
                *failing = false;
            }
            Err(e) if !*failing => {
                *failing = true;
                self.report(&StreamError::Failed(e));
            }
            Err(_) => {}
        }
    }

    async fn retry_uploads(&self) {
        for (name, result) in self.fs().upload_pending().await {
            match result {
                Ok(outcome) => self.log_outcome(&outcome),
                Err(e) => (self.events)(StreamEvent::UploadFailed {
                    account: self.account.email.to_string(),
                    name,
                    message: self.i18n.app_error(&e),
                }),
            }
        }
    }
}

impl StreamBackend for AccountStream {
    fn attr(&self, id: NodeId) -> Option<NodeAttr> {
        self.fs().attr(id)
    }

    async fn list(&self, dir: NodeId) -> Result<Vec<Entry>, StreamError> {
        let entries = self.fs().list(dir).await;
        self.wake_refresh();
        entries
    }

    async fn lookup(&self, dir: NodeId, name: &str) -> Result<Option<Entry>, StreamError> {
        let entry = self.fs().lookup(dir, name).await;
        self.wake_refresh();
        entry
    }

    async fn read(&self, id: NodeId, offset: u64, size: u32) -> Result<Vec<u8>, StreamError> {
        self.fs().read(id, offset, size).await
    }

    async fn create(&self, dir: NodeId, name: &str) -> Result<Entry, StreamError> {
        self.fs().create(dir, name).await
    }

    async fn open_write(&self, id: NodeId, truncate: bool) -> Result<(), StreamError> {
        self.fs().open_write(id, truncate).await
    }

    fn write(&self, id: NodeId, offset: u64, data: &[u8]) -> Result<u32, StreamError> {
        self.fs().write(id, offset, data)
    }

    async fn set_len(&self, id: NodeId, len: u64) -> Result<(), StreamError> {
        self.fs().set_len(id, len).await
    }

    async fn flush(&self, id: NodeId) -> Result<(), StreamError> {
        if let Some(outcome) = self.fs().flush(id).await? {
            self.log_outcome(&outcome);
        }
        Ok(())
    }

    async fn mkdir(&self, dir: NodeId, name: &str) -> Result<Entry, StreamError> {
        self.fs().mkdir(dir, name).await
    }

    async fn unlink(&self, dir: NodeId, name: &str) -> Result<(), StreamError> {
        self.fs().unlink(dir, name).await
    }

    async fn rmdir(&self, dir: NodeId, name: &str) -> Result<(), StreamError> {
        self.fs().rmdir(dir, name).await
    }

    async fn rename(
        &self,
        dir: NodeId,
        name: &str,
        new_dir: NodeId,
        new_name: &str,
    ) -> Result<(), StreamError> {
        self.fs().rename(dir, name, new_dir, new_name).await
    }

    async fn quota(&self) -> Option<StorageQuota> {
        if let Some((at, quota)) = *self.quota.lock().ok()?
            && at.elapsed() < QUOTA_TTL
        {
            return Some(quota);
        }
        let info = self
            .services
            .account_status()
            .execute(&self.account)
            .await
            .ok()?;
        *self.quota.lock().ok()? = Some((Instant::now(), info.quota));
        Some(info.quota)
    }

    fn report(&self, error: &StreamError) {
        let message = match error {
            StreamError::Failed(e) => self.i18n.app_error(e),
            other => format!("{other:?}"),
        };
        (self.events)(StreamEvent::Log(format!(
            "{}: {message}",
            self.account.email
        )));
    }
}

/// Opens the block cache in the XDG cache directory, shared by all accounts.
pub fn open_cache(i18n: &Localizer, limit: u64) -> Result<Arc<DiskBlockCache>, String> {
    let dir =
        nextgdrive_adapter_config::cache_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    DiskBlockCache::open(dir.join("blocks"), limit)
        .map(Arc::new)
        .map_err(|e| i18n.tr_args("error-storage", [("details", e.to_string().into())]))
}

/// A mounted account; dropping it stops retrying uploads and unmounts.
pub struct StreamMount {
    _mount: Mount,
    retry: tokio::task::JoinHandle<()>,
    refresh: tokio::task::JoinHandle<()>,
    index: tokio::task::JoinHandle<()>,
    stream: Arc<AccountStream>,
}

/// Lists offline folders in a mount ahead of time, so they can be browsed
/// without internet.
#[derive(Clone)]
pub struct Preloader(Arc<AccountStream>);

impl Preloader {
    /// Each item once per mount; failures are retried on the next call.
    pub async fn preload(&self, ids: &[nextgdrive_domain::DriveItemId]) {
        for id in ids {
            let done = self.0.preloaded.lock().is_ok_and(|p| p.contains(id));
            if done {
                continue;
            }
            if self.0.fs().preload(id).await.is_ok()
                && let Ok(mut preloaded) = self.0.preloaded.lock()
            {
                preloaded.insert(id.clone());
            }
        }
        self.0.wake_refresh();
    }
}

impl StreamMount {
    pub fn preloader(&self) -> Preloader {
        Preloader(self.stream.clone())
    }
}

impl Drop for StreamMount {
    fn drop(&mut self) {
        self.retry.abort();
        self.refresh.abort();
        self.index.abort();
    }
}

/// Mounts `account`'s Drive at `mountpoint`, answering on the current Tokio
/// runtime. Content written but not uploaded before (e.g. while offline, or
/// before a restart) is shown again and uploaded.
pub fn mount(
    services: Arc<Services>,
    cache: Arc<DiskBlockCache>,
    account: Account,
    mountpoint: &Path,
    i18n: Localizer,
    events: EventSink,
) -> Result<StreamMount, String> {
    let staging = open_staging(&i18n, &account)?;
    clear_stale_mount(mountpoint);
    let drive = IndexedDrive::new(
        services.drive.clone(),
        services.local.index.clone(),
        account.id.clone(),
    );
    let stream = Arc::new(AccountStream {
        drive,
        services,
        cache,
        staging: Arc::new(staging),
        account,
        state: StreamState::new(StreamConfig::default()),
        quota: Mutex::default(),
        events,
        i18n: i18n.clone(),
        preloaded: Mutex::default(),
        refresh: tokio::sync::Notify::new(),
    });
    let pending = stream.fs().recover().map_err(|e| i18n.app_error(&e))?;
    if pending > 0 {
        (stream.events)(StreamEvent::Log(
            i18n.tr_args("stream-pending", [("count", pending.into())]),
        ));
    }
    let mount = nextgdrive_adapter_fuse::mount(
        stream.clone(),
        mountpoint,
        tokio::runtime::Handle::current(),
    )
    .map_err(|e| {
        i18n.tr_args(
            "error-mount",
            [
                ("path", mountpoint.display().to_string().into()),
                ("details", e.to_string().into()),
            ],
        )
    })?;
    let retrying = stream.clone();
    let retry = tokio::spawn(async move {
        loop {
            retrying.retry_uploads().await;
            tokio::time::sleep(UPLOAD_CHECK_INTERVAL).await;
        }
    });
    let indexing = stream.clone();
    let index = tokio::spawn(async move {
        let mut failing = false;
        loop {
            indexing.update_index(&mut failing).await;
            let interval = indexing.services.local.get_settings().execute().map_or(
                nextgdrive_domain::Settings::DEFAULT_POLL_INTERVAL_SECS,
                |s| s.poll_interval_secs,
            );
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    });
    let refreshing = stream.clone();
    let refresh = tokio::spawn(async move {
        loop {
            refreshing.refresh.notified().await;
            refreshing.refresh_listings().await;
        }
    });
    Ok(StreamMount {
        _mount: mount,
        retry,
        refresh,
        index,
        stream,
    })
}

/// The account's streamed files not uploaded yet. One folder per account:
/// its pending files belong to its Drive.
pub fn open_staging(i18n: &Localizer, account: &Account) -> Result<DiskStaging, String> {
    let data =
        nextgdrive_adapter_config::data_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    DiskStaging::open(data.join("staging").join(account.id.as_str()))
        .map_err(|e| i18n.tr_args("error-storage", [("details", e.to_string().into())]))
}

/// After a crash the mount point is left dead ("Transport endpoint is not
/// connected") and cannot be mounted again until it is unmounted.
pub(crate) fn clear_stale_mount(mountpoint: &Path) {
    let dead = matches!(std::fs::metadata(mountpoint), Err(e) if e.raw_os_error() == Some(107)); // ENOTCONN
    if dead {
        let _ = std::process::Command::new(fusermount())
            .args(["-u", "-z"])
            .arg(mountpoint)
            .status();
    }
}

/// The `fusermount3` to run: `FUSERMOUNT_PATH` if set, as fuser does for the
/// mounts themselves. The Flatpak sets it to a wrapper that runs the host's,
/// since mounting needs a setuid helper the sandbox cannot have.
fn fusermount() -> std::ffi::OsString {
    std::env::var_os("FUSERMOUNT_PATH").unwrap_or_else(|| "fusermount3".into())
}

/// Whether a folder is empty (or missing), so a Drive can be mounted on it.
pub fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path).map_or(true, |mut entries| entries.next().is_none())
}

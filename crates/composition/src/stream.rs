//! Streaming mounts: an account's whole Drive as a folder, read on demand and
//! written through a staging area.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nextgdrive_adapter_cache::DiskBlockCache;
use nextgdrive_adapter_fuse::{Mount, StreamBackend};
use nextgdrive_adapter_localfs::DiskStaging;
use nextgdrive_application::streaming::{
    Entry, NodeAttr, NodeId, StreamConfig, StreamError, StreamState, StreamingDrive, UploadOutcome,
};
use nextgdrive_domain::{Account, StorageQuota};
use nextgdrive_i18n::Localizer;

use crate::Services;

/// Cached blocks of streamed files, shared by all accounts.
pub const DEFAULT_CACHE_LIMIT: u64 = 5 << 30;
const QUOTA_TTL: Duration = Duration::from_secs(5 * 60);
/// How often staged files are checked for uploading: new files once quiet
/// (`StreamConfig::new_file_quiet`), failed ones after their backoff.
const UPLOAD_CHECK_INTERVAL: Duration = Duration::from_secs(2);

type Log = Box<dyn Fn(String) + Send + Sync>;

struct AccountStream {
    services: Arc<Services>,
    cache: Arc<DiskBlockCache>,
    staging: Arc<DiskStaging>,
    account: Account,
    state: StreamState,
    quota: Mutex<Option<(Instant, StorageQuota)>>,
    log: Log,
    i18n: Localizer,
}

type Fs<'a> = StreamingDrive<
    'a,
    crate::GoogleAuthenticator,
    crate::KeyringCredentialStore,
    crate::GoogleDrive,
    DiskBlockCache,
    crate::TomlSettingsRepository,
    DiskStaging,
>;

impl AccountStream {
    fn fs(&self) -> Fs<'_> {
        StreamingDrive::new(
            &self.services.tokens,
            &self.services.drive,
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
            (self.log)(
                self.i18n
                    .tr_args("sync-conflict-copy", [("path", copy.clone().into())]),
            );
        }
    }

    async fn retry_uploads(&self) {
        for (name, result) in self.fs().upload_pending().await {
            match result {
                Ok(outcome) => self.log_outcome(&outcome),
                Err(e) => (self.log)(format!("{name}: {}", self.i18n.app_error(&e))),
            }
        }
    }
}

impl StreamBackend for AccountStream {
    fn attr(&self, id: NodeId) -> Option<NodeAttr> {
        self.fs().attr(id)
    }

    async fn list(&self, dir: NodeId) -> Result<Vec<Entry>, StreamError> {
        self.fs().list(dir).await
    }

    async fn lookup(&self, dir: NodeId, name: &str) -> Result<Option<Entry>, StreamError> {
        self.fs().lookup(dir, name).await
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
        (self.log)(format!("{}: {message}", self.account.email));
    }
}

/// Opens the block cache in the XDG cache directory.
pub fn open_cache(i18n: &Localizer) -> Result<Arc<DiskBlockCache>, String> {
    let dir =
        nextgdrive_adapter_config::cache_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    DiskBlockCache::open(dir.join("blocks"), DEFAULT_CACHE_LIMIT)
        .map(Arc::new)
        .map_err(|e| i18n.tr_args("error-storage", [("details", e.to_string().into())]))
}

/// A mounted account; dropping it stops retrying uploads and unmounts.
pub struct StreamMount {
    _mount: Mount,
    retry: tokio::task::JoinHandle<()>,
}

impl Drop for StreamMount {
    fn drop(&mut self) {
        self.retry.abort();
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
    log: Log,
) -> Result<StreamMount, String> {
    let storage_error = |e: String| i18n.tr_args("error-storage", [("details", e.into())]);
    let data =
        nextgdrive_adapter_config::data_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    // One staging folder per account: its pending files belong to its Drive.
    let staging = DiskStaging::open(data.join("staging").join(account.id.as_str()))
        .map_err(|e| storage_error(e.to_string()))?;
    let stream = Arc::new(AccountStream {
        services,
        cache,
        staging: Arc::new(staging),
        account,
        state: StreamState::new(StreamConfig::default()),
        quota: Mutex::default(),
        log,
        i18n: i18n.clone(),
    });
    let pending = stream.fs().recover().map_err(|e| i18n.app_error(&e))?;
    if pending > 0 {
        (stream.log)(i18n.tr_args("stream-pending", [("count", pending.into())]));
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
    let retry = tokio::spawn(async move {
        loop {
            stream.retry_uploads().await;
            tokio::time::sleep(UPLOAD_CHECK_INTERVAL).await;
        }
    });
    Ok(StreamMount {
        _mount: mount,
        retry,
    })
}

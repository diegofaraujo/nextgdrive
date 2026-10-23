//! Streaming mounts: an account's whole Drive as a folder, read on demand.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nextgdrive_adapter_cache::DiskBlockCache;
use nextgdrive_adapter_fuse::{Mount, StreamBackend};
use nextgdrive_application::AppError;
use nextgdrive_application::streaming::{
    Entry, NodeAttr, NodeId, StreamConfig, StreamState, StreamingDrive,
};
use nextgdrive_domain::{Account, StorageQuota};
use nextgdrive_i18n::Localizer;

use crate::Services;

/// Cached blocks of streamed files, shared by all accounts.
pub const DEFAULT_CACHE_LIMIT: u64 = 5 << 30;
const QUOTA_TTL: Duration = Duration::from_secs(5 * 60);

struct AccountStream {
    services: Arc<Services>,
    cache: Arc<DiskBlockCache>,
    account: Account,
    state: StreamState,
    quota: Mutex<Option<(Instant, StorageQuota)>>,
    log: Box<dyn Fn(String) + Send + Sync>,
    i18n: Localizer,
}

impl AccountStream {
    fn fs(
        &self,
    ) -> StreamingDrive<
        '_,
        crate::GoogleAuthenticator,
        crate::KeyringCredentialStore,
        crate::GoogleDrive,
        DiskBlockCache,
        crate::TomlSettingsRepository,
    > {
        StreamingDrive::new(
            &self.services.tokens,
            &self.services.drive,
            &*self.cache,
            &self.services.local.settings,
            &self.state,
            &self.account,
        )
    }
}

impl StreamBackend for AccountStream {
    fn attr(&self, id: NodeId) -> Option<NodeAttr> {
        self.fs().attr(id)
    }

    async fn list(&self, dir: NodeId) -> Result<Vec<Entry>, AppError> {
        self.fs().list(dir).await
    }

    async fn lookup(&self, dir: NodeId, name: &str) -> Result<Option<Entry>, AppError> {
        self.fs().lookup(dir, name).await
    }

    async fn read(&self, id: NodeId, offset: u64, size: u32) -> Result<Vec<u8>, AppError> {
        self.fs().read(id, offset, size).await
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

    fn report(&self, error: &AppError) {
        (self.log)(format!(
            "{}: {}",
            self.account.email,
            self.i18n.app_error(error)
        ));
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

/// Mounts `account`'s Drive at `mountpoint`, answering on the current Tokio
/// runtime. Dropping the returned mount unmounts it.
pub fn mount(
    services: Arc<Services>,
    cache: Arc<DiskBlockCache>,
    account: Account,
    mountpoint: &Path,
    i18n: Localizer,
    log: Box<dyn Fn(String) + Send + Sync>,
) -> Result<Mount, String> {
    let stream = AccountStream {
        services,
        cache,
        account,
        state: StreamState::new(StreamConfig::default()),
        quota: Mutex::default(),
        log,
        i18n: i18n.clone(),
    };
    nextgdrive_adapter_fuse::mount(
        Arc::new(stream),
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
    })
}

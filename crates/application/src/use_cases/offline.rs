//! Files and folders of a streamed account made available offline: kept in
//! full in the block cache, never evicted, and refreshed when they change.

use std::collections::{HashSet, VecDeque};
use std::path::{Component, Path};

use nextgdrive_domain::local_name::{assign_names, local_name};
use nextgdrive_domain::{Account, DriveItem, DriveItemId, DriveKind, ItemKind};

use crate::ports::{
    AccessToken, Authenticator, BlockCache, BlockKey, CredentialStore, DriveService, PortError,
    SettingsRepository,
};
use crate::streaming::version_of;
use crate::{AppError, TokenProvider};

/// Blocks fetched per request when downloading for offline use.
const BLOCKS_PER_REQUEST: u64 = 8;

/// A file to keep in the cache, at its current version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineFile {
    pub id: DriveItemId,
    pub version: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheUsage {
    pub used: u64,
    /// Part of `used` that belongs to offline items (never evicted).
    pub offline: u64,
}

/// An item marked available offline, and how much of it is on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineItem {
    pub item: DriveItem,
    pub files: usize,
    pub bytes_total: u64,
    pub bytes_cached: u64,
}

impl OfflineItem {
    pub fn complete(&self) -> bool {
        self.bytes_cached >= self.bytes_total
    }
}

pub struct Offline<'a, A, C, D, K, S> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    cache: &'a K,
    settings: &'a S,
    /// Must be the streaming block size, so both use the same cached blocks.
    block_size: u64,
}

impl<'a, A, C, D, K, S> Offline<'a, A, C, D, K, S>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    K: BlockCache,
    S: SettingsRepository,
{
    pub fn new(
        tokens: &'a TokenProvider<A, C>,
        drive: &'a D,
        cache: &'a K,
        settings: &'a S,
        block_size: u64,
    ) -> Self {
        Self {
            tokens,
            drive,
            cache,
            settings,
            block_size,
        }
    }

    /// Marks the item at `path` (relative to the account folder, e.g.
    /// `My Drive/Docs/report.pdf`) available offline.
    pub async fn add(&self, account: &Account, path: &Path) -> Result<DriveItem, AppError> {
        let token = self.tokens.access_token(account).await?;
        let item = self.resolve(&token, path).await?;
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        settings
            .offline
            .entry(account.id.clone())
            .or_default()
            .insert(item.id.clone());
        self.settings.save(&settings).map_err(AppError::Storage)?;
        Ok(item)
    }

    /// Unmarks the item at `path` (relative to the account folder).
    pub async fn remove_path(&self, account: &Account, path: &Path) -> Result<DriveItem, AppError> {
        let token = self.tokens.access_token(account).await?;
        let item = self.resolve(&token, path).await?;
        self.remove(account, &item.id)?;
        Ok(item)
    }

    /// Its blocks become ordinary cache entries again.
    pub fn remove(&self, account: &Account, id: &DriveItemId) -> Result<(), AppError> {
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        if let Some(items) = settings.offline.get_mut(&account.id) {
            items.remove(id);
            if items.is_empty() {
                settings.offline.remove(&account.id);
            }
        }
        self.settings.save(&settings).map_err(AppError::Storage)
    }

    /// Every file under the account's offline items, at its current version.
    pub async fn files(&self, account: &Account) -> Result<Vec<OfflineFile>, AppError> {
        Ok(self.status_and_files(account).await?.1)
    }

    /// Each offline item, with how much of it is cached.
    pub async fn status(&self, account: &Account) -> Result<Vec<OfflineItem>, AppError> {
        Ok(self.status_and_files(account).await?.0)
    }

    /// Downloads the blocks of `files` that are not cached yet. Returns the
    /// bytes downloaded.
    pub async fn download(
        &self,
        account: &Account,
        files: &[OfflineFile],
    ) -> Result<u64, AppError> {
        let token = self.tokens.access_token(account).await?;
        let mut downloaded = 0;
        for file in files {
            let blocks = file.size.div_ceil(self.block_size);
            let mut index = 0;
            while index < blocks {
                if self.cache.contains(&self.key(file, index)) {
                    index += 1;
                    continue;
                }
                // A run of missing blocks, fetched in one request.
                let mut count = 1;
                while count < BLOCKS_PER_REQUEST
                    && index + count < blocks
                    && !self.cache.contains(&self.key(file, index + count))
                {
                    count += 1;
                }
                let data = self
                    .drive
                    .download_range(
                        &token,
                        &file.id,
                        index * self.block_size,
                        count * self.block_size,
                    )
                    .await
                    .map_err(AppError::Drive)?;
                let expected = (file.size - index * self.block_size).min(count * self.block_size);
                if data.len() as u64 != expected {
                    // Changed meanwhile; the next round fetches the new version.
                    break;
                }
                let block = usize::try_from(self.block_size).unwrap_or(usize::MAX);
                for (n, chunk) in data.chunks(block).enumerate() {
                    self.cache
                        .put(&self.key(file, index + n as u64), chunk)
                        .map_err(AppError::Storage)?;
                }
                downloaded += data.len() as u64;
                index += count;
            }
        }
        Ok(downloaded)
    }

    /// Empties the cache of streamed files, except what `accounts` keep
    /// offline. Nothing in Drive is touched; removed content is downloaded
    /// again when opened. Returns the bytes freed.
    pub async fn clear_cache(&self, accounts: &[Account]) -> Result<u64, AppError> {
        let mut keep = Vec::new();
        for account in accounts {
            keep.extend(
                self.files(account)
                    .await?
                    .into_iter()
                    .map(|f| (f.id, f.version)),
            );
        }
        self.cache.clear_except(&keep).map_err(AppError::Storage)
    }

    /// Bytes in the cache, and how many of them are offline items of `accounts`.
    pub async fn cache_usage(&self, accounts: &[Account]) -> Result<CacheUsage, AppError> {
        let mut offline = 0;
        for account in accounts {
            offline += self
                .status(account)
                .await?
                .iter()
                .map(|i| i.bytes_cached)
                .sum::<u64>();
        }
        Ok(CacheUsage {
            used: self.cache.usage(),
            offline,
        })
    }

    fn key(&self, file: &OfflineFile, index: u64) -> BlockKey {
        BlockKey {
            file: file.id.clone(),
            version: file.version.clone(),
            index,
        }
    }

    fn cached_bytes(&self, file: &OfflineFile) -> u64 {
        (0..file.size.div_ceil(self.block_size))
            .filter(|index| self.cache.contains(&self.key(file, *index)))
            .map(|index| (file.size - index * self.block_size).min(self.block_size))
            .sum()
    }

    async fn status_and_files(
        &self,
        account: &Account,
    ) -> Result<(Vec<OfflineItem>, Vec<OfflineFile>), AppError> {
        let settings = self.settings.load().map_err(AppError::Storage)?;
        let ids: Vec<DriveItemId> = settings.offline_items(&account.id).cloned().collect();
        if ids.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let token = self.tokens.access_token(account).await?;
        let mut items = Vec::new();
        let mut all = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            // An item deleted in Drive since is just skipped.
            let Ok(item) = self.drive.item(&token, &id).await else {
                continue;
            };
            let files = self.files_under(&token, &item).await?;
            items.push(OfflineItem {
                files: files.len(),
                bytes_total: files.iter().map(|f| f.size).sum(),
                bytes_cached: files.iter().map(|f| self.cached_bytes(f)).sum(),
                item,
            });
            all.extend(files.into_iter().filter(|f| seen.insert(f.id.clone())));
        }
        Ok((items, all))
    }

    async fn files_under(
        &self,
        token: &AccessToken,
        item: &DriveItem,
    ) -> Result<Vec<OfflineFile>, AppError> {
        let mut files = Vec::new();
        let mut queue = VecDeque::from([item.clone()]);
        while let Some(item) = queue.pop_front() {
            match &item.kind {
                ItemKind::File { size, .. } => files.push(OfflineFile {
                    version: version_of(&item),
                    size: *size,
                    id: item.id,
                }),
                ItemKind::Folder => {
                    let children = self
                        .drive
                        .children(token, &item.id)
                        .await
                        .map_err(AppError::Drive)?;
                    queue.extend(children);
                }
                // Google Docs' shortcuts are generated: nothing to download.
                _ => {}
            }
        }
        Ok(files)
    }

    /// The Drive item at a path relative to the account folder, following the
    /// same naming as the streamed tree.
    async fn resolve(&self, token: &AccessToken, path: &Path) -> Result<DriveItem, AppError> {
        let not_found = || {
            AppError::Drive(PortError::new(format!(
                "not in this Google Drive: {}",
                path.display()
            )))
        };
        let settings = self.settings.load().map_err(AppError::Storage)?;
        let mut parts = path.components().filter_map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        });
        let drives = self.drive.drives(token).await.map_err(AppError::Drive)?;
        let top = parts.next().ok_or_else(not_found)?;
        let root = if top == settings.my_drive_folder {
            drives
                .iter()
                .find(|d| d.kind == DriveKind::MyDrive)
                .map(|d| d.root.clone())
        } else if top == settings.shared_drives_folder {
            let name = parts.next().ok_or_else(not_found)?;
            let shared: Vec<(DriveItemId, String)> = drives
                .iter()
                .filter_map(|d| match &d.kind {
                    DriveKind::Shared { name } => {
                        Some((d.root.clone(), local_name(name, &ItemKind::Folder)))
                    }
                    DriveKind::MyDrive => None,
                })
                .collect();
            let names = assign_names(shared.iter().map(|(id, n)| (id, n.clone())));
            names.into_iter().find(|(_, n)| n == name).map(|(id, _)| id)
        } else {
            None
        };
        let mut current = self
            .drive
            .item(token, &root.ok_or_else(not_found)?)
            .await
            .map_err(AppError::Drive)?;
        for name in parts {
            let children = self
                .drive
                .children(token, &current.id)
                .await
                .map_err(AppError::Drive)?;
            let shown: Vec<&DriveItem> = children
                .iter()
                .filter(|i| {
                    matches!(
                        i.kind,
                        ItemKind::Folder | ItemKind::File { .. } | ItemKind::GoogleDoc { .. }
                    )
                })
                .collect();
            let names = assign_names(shown.iter().map(|i| (&i.id, local_name(&i.name, &i.kind))));
            let id = names
                .into_iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| id)
                .ok_or_else(not_found)?;
            current = children
                .into_iter()
                .find(|i| i.id == id)
                .ok_or_else(not_found)?;
        }
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, InMemoryBlocks, InMemoryCredentials, InMemorySettings, account,
    };

    struct World {
        ana: Account,
        tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
        drive: FakeDrive,
        cache: InMemoryBlocks,
        settings: InMemorySettings,
    }

    fn world() -> World {
        let ana = account("1", "ana@x.com");
        World {
            tokens: TokenProvider::new(
                FakeAuth::default(),
                InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
            ),
            drive: FakeDrive::new(ana.clone())
                .with_folder("root", "docs", "Docs")
                .with_content("docs", "a", "a.txt", "0123456789abcdefghij")
                .with_folder("docs", "sub", "Sub")
                .with_content("sub", "b", "b.txt", "beta")
                .with_content("root", "c", "c.txt", "gamma"),
            ana,
            cache: InMemoryBlocks::default(),
            settings: InMemorySettings::default(),
        }
    }

    impl World {
        fn offline(
            &self,
        ) -> Offline<'_, FakeAuth, InMemoryCredentials, FakeDrive, InMemoryBlocks, InMemorySettings>
        {
            Offline::new(&self.tokens, &self.drive, &self.cache, &self.settings, 8)
        }
    }

    #[tokio::test]
    async fn adds_items_by_their_local_path() {
        let w = world();

        let docs = w
            .offline()
            .add(&w.ana, Path::new("My Drive/Docs"))
            .await
            .unwrap();
        let b = w
            .offline()
            .add(&w.ana, Path::new("My Drive/Docs/Sub/b.txt"))
            .await
            .unwrap();

        assert_eq!((docs.id.as_str(), b.id.as_str()), ("docs", "b"));
        let settings = w.settings.load().unwrap();
        let ids: Vec<_> = settings
            .offline_items(&w.ana.id)
            .map(ToString::to_string)
            .collect();
        assert_eq!(ids, ["b", "docs"]);
        assert!(
            w.offline()
                .add(&w.ana, Path::new("My Drive/nope.txt"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn downloads_everything_under_a_folder_once() {
        let w = world();
        w.offline()
            .add(&w.ana, Path::new("My Drive/Docs"))
            .await
            .unwrap();

        let files = w.offline().files(&w.ana).await.unwrap();
        let downloaded = w.offline().download(&w.ana, &files).await.unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(downloaded, 24);
        let status = w.offline().status(&w.ana).await.unwrap();
        assert!(status[0].complete(), "{status:?}");
        let calls = w.drive.range_calls.load(Ordering::Relaxed);
        assert_eq!(w.offline().download(&w.ana, &files).await.unwrap(), 0);
        assert_eq!(
            w.drive.range_calls.load(Ordering::Relaxed),
            calls,
            "nothing fetched again"
        );
    }

    #[tokio::test]
    async fn a_changed_file_is_fetched_again_at_its_new_version() {
        let w = world();
        w.offline()
            .add(&w.ana, Path::new("My Drive/c.txt"))
            .await
            .unwrap();
        let files = w.offline().files(&w.ana).await.unwrap();
        w.offline().download(&w.ana, &files).await.unwrap();

        w.drive.set_content("c", "gamma v2");
        let files = w.offline().files(&w.ana).await.unwrap();

        assert!(!w.offline().status(&w.ana).await.unwrap()[0].complete());
        assert_eq!(w.offline().download(&w.ana, &files).await.unwrap(), 8);
    }

    #[tokio::test]
    async fn the_streamed_folder_reads_offline_files_without_the_network() {
        use crate::streaming::{ROOT, StreamConfig, StreamState, StreamingDrive};
        use crate::use_cases::fakes::InMemoryStaging;

        let w = world();
        w.offline()
            .add(&w.ana, Path::new("My Drive/Docs"))
            .await
            .unwrap();
        let files = w.offline().files(&w.ana).await.unwrap();
        w.offline().download(&w.ana, &files).await.unwrap();
        let calls = w.drive.range_calls.load(Ordering::Relaxed);

        let state = StreamState::new(StreamConfig {
            block_size: 8,
            ..StreamConfig::default()
        });
        let staging = InMemoryStaging::default();
        let fs = StreamingDrive::new(
            &w.tokens,
            &w.drive,
            &w.cache,
            &w.settings,
            &staging,
            &state,
            &w.ana,
            String::new(),
        );
        let mut dir = ROOT;
        for name in ["My Drive", "Docs", "a.txt"] {
            dir = fs.lookup(dir, name).await.unwrap().unwrap().attr.id;
        }

        assert_eq!(fs.read(dir, 0, 100).await.unwrap(), b"0123456789abcdefghij");
        assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), calls);
    }

    #[tokio::test]
    async fn clearing_the_cache_keeps_offline_items() {
        use crate::streaming::{ROOT, StreamConfig, StreamState, StreamingDrive};
        use crate::use_cases::fakes::InMemoryStaging;

        let w = world();
        w.offline()
            .add(&w.ana, Path::new("My Drive/Docs/Sub"))
            .await
            .unwrap();
        let files = w.offline().files(&w.ana).await.unwrap();
        w.offline().download(&w.ana, &files).await.unwrap();
        // Something read while streaming, not kept offline.
        let state = StreamState::new(StreamConfig {
            block_size: 8,
            ..StreamConfig::default()
        });
        let staging = InMemoryStaging::default();
        let fs = StreamingDrive::new(
            &w.tokens,
            &w.drive,
            &w.cache,
            &w.settings,
            &staging,
            &state,
            &w.ana,
            String::new(),
        );
        let c = fs
            .lookup(
                fs.lookup(ROOT, "My Drive").await.unwrap().unwrap().attr.id,
                "c.txt",
            )
            .await
            .unwrap()
            .unwrap();
        fs.read(c.attr.id, 0, 100).await.unwrap();
        let usage = w
            .offline()
            .cache_usage(std::slice::from_ref(&w.ana))
            .await
            .unwrap();
        assert_eq!((usage.used, usage.offline), (9, 4));

        let freed = w
            .offline()
            .clear_cache(std::slice::from_ref(&w.ana))
            .await
            .unwrap();

        assert_eq!(freed, 5);
        assert!(
            w.offline().status(&w.ana).await.unwrap()[0].complete(),
            "offline item untouched"
        );
    }

    #[tokio::test]
    async fn removing_forgets_the_item() {
        let w = world();
        let c = w
            .offline()
            .add(&w.ana, Path::new("My Drive/c.txt"))
            .await
            .unwrap();
        w.offline().remove(&w.ana, &c.id).unwrap();
        assert!(w.offline().files(&w.ana).await.unwrap().is_empty());
    }
}

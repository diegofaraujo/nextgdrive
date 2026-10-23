//! Streaming: an account's whole Drive shown as a folder tree whose file
//! contents are fetched only when read, in blocks kept in a cache.
//!
//! This is the logic behind the virtual filesystem; the FUSE adapter only
//! translates kernel requests into these calls. Read-only for now.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use nextgdrive_domain::local_name::{assign_names, google_doc_shortcut, local_name};
use nextgdrive_domain::{Account, DriveItem, DriveItemId, DriveKind, ItemKind};

use crate::ports::{
    Authenticator, BlockCache, BlockKey, CredentialStore, DriveService, PortError,
    SettingsRepository,
};
use crate::{AppError, TokenProvider};

/// Identifies a node for the lifetime of a mount (the FUSE inode number).
pub type NodeId = u64;

/// The account folder itself.
pub const ROOT: NodeId = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAttr {
    pub id: NodeId,
    pub kind: NodeKind,
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub attr: NodeAttr,
}

#[derive(Debug, Clone, Copy)]
pub struct StreamConfig {
    /// How long a folder listing is trusted before asking Drive again.
    pub listing_ttl: Duration,
    /// Content is fetched and cached in blocks of this many bytes.
    pub block_size: u64,
    /// While a file is read sequentially, a missing block is fetched together
    /// with the next ones, up to this many, in a single request.
    pub readahead_blocks: u64,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            listing_ttl: Duration::from_secs(30),
            block_size: 1 << 20,
            readahead_blocks: 8,
        }
    }
}

#[derive(Debug, Clone)]
enum Content {
    /// The account folder: My Drive and, if any, the shared drives folder.
    Root,
    SharedDrives,
    Folder(DriveItemId),
    File {
        id: DriveItemId,
        size: u64,
        version: String,
    },
    /// A Google Doc, shown as its generated `.html` shortcut.
    Shortcut(Vec<u8>),
}

#[derive(Debug, Clone)]
struct Node {
    content: Content,
    modified: SystemTime,
}

#[derive(Default)]
struct Table {
    nodes: HashMap<NodeId, Node>,
    /// (parent, what the child is) → node, so a node keeps its id when its
    /// folder is listed again, even if it was renamed.
    by_key: HashMap<(NodeId, String), NodeId>,
    /// Children of a listed folder, with their names, and when it was listed.
    listings: HashMap<NodeId, (Instant, Vec<(String, NodeId)>)>,
    /// Where the last read of each file ended, to recognize sequential reads.
    read_ends: HashMap<NodeId, u64>,
    next: NodeId,
}

/// What a mount remembers between requests. Shared by all of them.
pub struct StreamState {
    config: StreamConfig,
    /// Shown as the modification time of folders Drive gives none for.
    mounted_at: SystemTime,
    table: Mutex<Table>,
}

impl StreamState {
    pub fn new(config: StreamConfig) -> Self {
        let mounted_at = SystemTime::now();
        let mut table = Table {
            next: ROOT + 1,
            ..Table::default()
        };
        table.nodes.insert(
            ROOT,
            Node {
                content: Content::Root,
                modified: mounted_at,
            },
        );
        Self {
            config,
            mounted_at,
            table: Mutex::new(table),
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().expect("stream table poisoned")
    }
}

/// One request against a mounted account.
pub struct StreamingDrive<'a, A, C, D, K, S> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    cache: &'a K,
    settings: &'a S,
    state: &'a StreamState,
    account: &'a Account,
}

impl<'a, A, C, D, K, S> StreamingDrive<'a, A, C, D, K, S>
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
        state: &'a StreamState,
        account: &'a Account,
    ) -> Self {
        Self {
            tokens,
            drive,
            cache,
            settings,
            state,
            account,
        }
    }

    /// `None` for ids this mount never handed out.
    pub fn attr(&self, id: NodeId) -> Option<NodeAttr> {
        let table = self.state.table();
        table.nodes.get(&id).map(|node| attr_of(id, node))
    }

    pub async fn list(&self, dir: NodeId) -> Result<Vec<Entry>, AppError> {
        let fresh = {
            let table = self.state.table();
            table
                .listings
                .get(&dir)
                .filter(|(at, _)| at.elapsed() < self.state.config.listing_ttl)
                .map(|(_, children)| children.clone())
        };
        let children = match fresh {
            Some(children) => children,
            None => self.fetch_listing(dir).await?,
        };
        let table = self.state.table();
        Ok(children
            .into_iter()
            .filter_map(|(name, id)| {
                let node = table.nodes.get(&id)?;
                Some(Entry {
                    name,
                    attr: attr_of(id, node),
                })
            })
            .collect())
    }

    pub async fn lookup(&self, dir: NodeId, name: &str) -> Result<Option<Entry>, AppError> {
        Ok(self.list(dir).await?.into_iter().find(|e| e.name == name))
    }

    /// Up to `size` bytes from `offset`, fetching only the blocks not cached yet.
    pub async fn read(&self, node: NodeId, offset: u64, size: u32) -> Result<Vec<u8>, AppError> {
        let content = {
            let table = self.state.table();
            table.nodes.get(&node).map(|n| n.content.clone())
        };
        match content {
            Some(Content::Shortcut(bytes)) => {
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                let end = start.saturating_add(size as usize).min(bytes.len());
                Ok(bytes[start..end].to_vec())
            }
            Some(Content::File {
                id,
                size: file_size,
                version,
            }) => {
                let end = offset.saturating_add(u64::from(size)).min(file_size);
                if offset >= end {
                    return Ok(Vec::new());
                }
                let config = self.state.config;
                let sequential = {
                    let mut table = self.state.table();
                    let previous = table.read_ends.insert(node, end);
                    previous == Some(offset) && offset > 0
                };
                let ahead = if sequential {
                    config.readahead_blocks.max(1)
                } else {
                    1
                };
                let block_size = config.block_size;
                let mut out = Vec::with_capacity(usize::try_from(end - offset).unwrap_or(0));
                for index in offset / block_size..=(end - 1) / block_size {
                    let block = self.block(&id, &version, index, file_size, ahead).await?;
                    let block_start = index * block_size;
                    let from = usize::try_from(offset.max(block_start) - block_start).unwrap_or(0);
                    let to = usize::try_from(end.min(block_start + block_size) - block_start)
                        .unwrap_or(0);
                    out.extend_from_slice(&block[from.min(block.len())..to.min(block.len())]);
                }
                Ok(out)
            }
            _ => Err(AppError::Drive(PortError::new("not a file"))),
        }
    }

    /// Block `index`, from the cache or else from Drive together with the
    /// following blocks, `ahead` in all (fewer at the end of the file).
    async fn block(
        &self,
        id: &DriveItemId,
        version: &str,
        index: u64,
        file_size: u64,
        ahead: u64,
    ) -> Result<Vec<u8>, AppError> {
        let block_size = self.state.config.block_size;
        let key = |index| BlockKey {
            file: id.clone(),
            version: version.to_owned(),
            index,
        };
        let expected = |index: u64| file_size.saturating_sub(index * block_size).min(block_size);
        // A short block (e.g. a cache entry cut off by a crash) is refetched.
        if let Some(block) = self.cache.get(&key(index)).map_err(AppError::Storage)?
            && block.len() as u64 == expected(index)
        {
            return Ok(block);
        }
        let last = file_size.saturating_sub(1) / block_size;
        let count = ahead.min(last.saturating_sub(index) + 1);
        let token = self.tokens.access_token(self.account).await?;
        let data = self
            .drive
            .download_range(&token, id, index * block_size, count * block_size)
            .await
            .map_err(AppError::Drive)?;
        let wanted: u64 = (index..index + count).map(expected).sum();
        if data.len() as u64 != wanted {
            return Err(AppError::Drive(PortError::new(
                "file changed while being read",
            )));
        }
        let mut first = None;
        for (n, chunk) in data
            .chunks(usize::try_from(block_size).unwrap_or(usize::MAX))
            .enumerate()
        {
            self.cache
                .put(&key(index + n as u64), chunk)
                .map_err(AppError::Storage)?;
            first.get_or_insert_with(|| chunk.to_vec());
        }
        Ok(first.unwrap_or_default())
    }

    async fn fetch_listing(&self, dir: NodeId) -> Result<Vec<(String, NodeId)>, AppError> {
        let content = {
            let table = self.state.table();
            table.nodes.get(&dir).map(|n| n.content.clone())
        };
        let token = self.tokens.access_token(self.account).await?;
        // (key, display name, content, modified)
        let mut children: Vec<(String, String, Content, SystemTime)> = Vec::new();
        match content {
            Some(Content::Root) => {
                let settings = self.settings.load().map_err(AppError::Storage)?;
                let drives = self.drive.drives(&token).await.map_err(AppError::Drive)?;
                for drive in &drives {
                    if drive.kind == DriveKind::MyDrive {
                        children.push((
                            "my-drive".into(),
                            settings.my_drive_folder.clone(),
                            Content::Folder(drive.root.clone()),
                            self.state.mounted_at,
                        ));
                    }
                }
                if drives
                    .iter()
                    .any(|d| matches!(d.kind, DriveKind::Shared { .. }))
                {
                    children.push((
                        "shared-drives".into(),
                        settings.shared_drives_folder.clone(),
                        Content::SharedDrives,
                        self.state.mounted_at,
                    ));
                }
            }
            Some(Content::SharedDrives) => {
                let drives = self.drive.drives(&token).await.map_err(AppError::Drive)?;
                let shared: Vec<(DriveItemId, String)> = drives
                    .into_iter()
                    .filter_map(|d| match d.kind {
                        DriveKind::Shared { name } => Some((d.root, name)),
                        DriveKind::MyDrive => None,
                    })
                    .collect();
                let names = assign_names(
                    shared
                        .iter()
                        .map(|(id, name)| (id, local_name(name, &ItemKind::Folder))),
                );
                for (id, _) in shared {
                    children.push((
                        id.to_string(),
                        names[&id].clone(),
                        Content::Folder(id),
                        self.state.mounted_at,
                    ));
                }
            }
            Some(Content::Folder(folder)) => {
                let items = self
                    .drive
                    .children(&token, &folder)
                    .await
                    .map_err(AppError::Drive)?;
                let shown: Vec<DriveItem> = items
                    .into_iter()
                    .filter(|i| content_of(i).is_some())
                    .collect();
                let names =
                    assign_names(shown.iter().map(|i| (&i.id, local_name(&i.name, &i.kind))));
                for item in shown {
                    let content = content_of(&item).expect("filtered above");
                    children.push((
                        item.id.to_string(),
                        names[&item.id].clone(),
                        content,
                        item.modified,
                    ));
                }
            }
            _ => return Err(AppError::Drive(PortError::new("not a folder"))),
        }

        let mut table = self.state.table();
        let mut listing = Vec::with_capacity(children.len());
        for (key, name, content, modified) in children {
            let id = match table.by_key.get(&(dir, key.clone())) {
                Some(id) => *id,
                None => {
                    let id = table.next;
                    table.next += 1;
                    table.by_key.insert((dir, key), id);
                    id
                }
            };
            table.nodes.insert(id, Node { content, modified });
            listing.push((name, id));
        }
        table
            .listings
            .insert(dir, (Instant::now(), listing.clone()));
        Ok(listing)
    }
}

/// How a Drive item appears in the tree; `None` for items not shown.
fn content_of(item: &DriveItem) -> Option<Content> {
    match &item.kind {
        ItemKind::Folder => Some(Content::Folder(item.id.clone())),
        ItemKind::File { size, md5 } => Some(Content::File {
            id: item.id.clone(),
            size: *size,
            // Without an MD5, the modification time stands for the version.
            version: md5.clone().unwrap_or_else(|| {
                let nanos = item
                    .modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                format!("t{nanos}")
            }),
        }),
        ItemKind::GoogleDoc { url } => Some(Content::Shortcut(
            google_doc_shortcut(&item.name, url).into_bytes(),
        )),
        ItemKind::Shortcut | ItemKind::NotDownloadable => None,
    }
}

fn attr_of(id: NodeId, node: &Node) -> NodeAttr {
    let (kind, size) = match &node.content {
        Content::Root | Content::SharedDrives | Content::Folder(_) => (NodeKind::Dir, 0),
        Content::File { size, .. } => (NodeKind::File, *size),
        Content::Shortcut(bytes) => (NodeKind::File, bytes.len() as u64),
    };
    NodeAttr {
        id,
        kind,
        size,
        modified: node.modified,
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
        account: Account,
        tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
        drive: FakeDrive,
        cache: InMemoryBlocks,
        settings: InMemorySettings,
        state: StreamState,
    }

    fn world(listing_ttl: Duration) -> World {
        let account = account("1", "ana@x.com");
        World {
            tokens: TokenProvider::new(
                FakeAuth::default(),
                InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
            ),
            drive: FakeDrive::new(account.clone())
                .with_folder("root", "docs", "Docs")
                .with_content("docs", "a", "a.txt", "0123456789abcdefghij")
                .with_item(
                    "docs",
                    "sheet",
                    "Budget",
                    ItemKind::GoogleDoc {
                        url: "https://docs.google.com/spreadsheets/d/sheet".into(),
                    },
                )
                .with_item("docs", "link", "link", ItemKind::Shortcut),
            account,
            cache: InMemoryBlocks::default(),
            settings: InMemorySettings::default(),
            // Tiny blocks, so reads span several.
            state: StreamState::new(StreamConfig {
                listing_ttl,
                block_size: 8,
                readahead_blocks: 2,
            }),
        }
    }

    impl World {
        fn fs(
            &self,
        ) -> StreamingDrive<
            '_,
            FakeAuth,
            InMemoryCredentials,
            FakeDrive,
            InMemoryBlocks,
            InMemorySettings,
        > {
            StreamingDrive::new(
                &self.tokens,
                &self.drive,
                &self.cache,
                &self.settings,
                &self.state,
                &self.account,
            )
        }

        async fn path(&self, path: &[&str]) -> Entry {
            let mut dir = ROOT;
            let mut entry = None;
            for name in path {
                let found = self
                    .fs()
                    .lookup(dir, name)
                    .await
                    .unwrap()
                    .unwrap_or_else(|| panic!("{name}"));
                dir = found.attr.id;
                entry = Some(found);
            }
            entry.expect("non-empty path")
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[tokio::test]
    async fn shows_the_drive_without_downloading_anything() {
        let w = world(Duration::from_secs(60));

        assert_eq!(names(&w.fs().list(ROOT).await.unwrap()), ["My Drive"]);
        let docs = w.path(&["My Drive", "Docs"]).await;
        let entries = w.fs().list(docs.attr.id).await.unwrap();

        assert_eq!(names(&entries), ["a.txt", "Budget.html"]);
        assert_eq!(entries[0].attr.size, 20);
        assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn reads_fetch_only_the_blocks_needed_and_only_once() {
        let w = world(Duration::from_secs(60));
        let a = w.path(&["My Drive", "Docs", "a.txt"]).await;

        // Bytes 6..10 span blocks 0 and 1.
        assert_eq!(w.fs().read(a.attr.id, 6, 4).await.unwrap(), b"6789");
        assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 2);

        assert_eq!(
            w.fs().read(a.attr.id, 0, 100).await.unwrap(),
            b"0123456789abcdefghij"
        );
        // Only the last block was new.
        assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 3);
        assert_eq!(w.fs().read(a.attr.id, 20, 5).await.unwrap(), b"");
    }

    #[tokio::test]
    async fn sequential_reads_fetch_ahead_in_one_request() {
        let w = world(Duration::from_secs(60));
        let a = w.path(&["My Drive", "Docs", "a.txt"]).await;
        let calls = || w.drive.range_calls.load(Ordering::Relaxed);

        // The first read is not known to be sequential: one block.
        assert_eq!(w.fs().read(a.attr.id, 0, 4).await.unwrap(), b"0123");
        assert_eq!(calls(), 1);
        // Continuing where it ended: blocks 1 and 2 come in one request.
        assert_eq!(w.fs().read(a.attr.id, 4, 8).await.unwrap(), b"456789ab");
        assert_eq!(calls(), 2);
        assert_eq!(w.fs().read(a.attr.id, 12, 8).await.unwrap(), b"cdefghij");
        assert_eq!(calls(), 2, "already fetched ahead");
    }

    #[tokio::test]
    async fn new_versions_never_get_stale_blocks() {
        let w = world(Duration::ZERO);
        let a = w.path(&["My Drive", "Docs", "a.txt"]).await;
        w.fs().read(a.attr.id, 0, 4).await.unwrap();

        w.drive.set_content("a", "NEW CONTENT");
        let again = w.path(&["My Drive", "Docs", "a.txt"]).await;

        assert_eq!(again.attr.id, a.attr.id, "same file keeps its node");
        assert_eq!(again.attr.size, 11);
        assert_eq!(w.fs().read(a.attr.id, 0, 4).await.unwrap(), b"NEW ");
    }

    #[tokio::test]
    async fn google_docs_read_as_their_shortcut() {
        let w = world(Duration::from_secs(60));
        let doc = w.path(&["My Drive", "Docs", "Budget.html"]).await;

        let page = w.fs().read(doc.attr.id, 0, 10_000).await.unwrap();

        assert_eq!(page.len() as u64, doc.attr.size);
        assert!(
            String::from_utf8(page)
                .unwrap()
                .contains("docs.google.com/spreadsheets/d/sheet")
        );
    }

    #[tokio::test]
    async fn renamed_items_keep_their_node() {
        let w = world(Duration::ZERO);
        let a = w.path(&["My Drive", "Docs", "a.txt"]).await;

        w.drive.rename("a", "renamed.txt");
        let docs = w.path(&["My Drive", "Docs"]).await;

        assert!(
            w.fs()
                .lookup(docs.attr.id, "a.txt")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            w.path(&["My Drive", "Docs", "renamed.txt"]).await.attr.id,
            a.attr.id
        );
    }

    #[tokio::test]
    async fn listings_are_reused_until_they_expire() {
        let w = world(Duration::from_secs(60));
        let docs = w.path(&["My Drive", "Docs"]).await;
        w.fs().list(docs.attr.id).await.unwrap();

        w.drive.rename("a", "renamed.txt");

        assert_eq!(names(&w.fs().list(docs.attr.id).await.unwrap())[0], "a.txt");
    }
}

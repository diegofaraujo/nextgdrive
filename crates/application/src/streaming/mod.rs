//! Streaming: an account's whole Drive shown as a folder tree whose file
//! contents are fetched only when read, in blocks kept in a cache.
//!
//! This is the logic behind the virtual filesystem; the FUSE adapter only
//! translates kernel requests into these calls. Writes (see `write.rs`) go
//! to a staging area first and are uploaded when the file is closed.

mod trash_dir;
mod write;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use nextgdrive_domain::local_name::{assign_names, google_doc_shortcut, local_name};
use nextgdrive_domain::{Account, DriveItem, DriveItemId, DriveKind, ItemKind};

use crate::ports::{
    Authenticator, BlockCache, BlockKey, CredentialStore, DriveService, PortError,
    SettingsRepository, StagedFile, StagedTarget, StagingArea,
};
use crate::{AppError, TokenProvider};

pub use write::UploadOutcome;

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
    /// Only the owner may enter it (the desktop trash requires that).
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub attr: NodeAttr,
}

/// Why a filesystem request failed; the FUSE adapter turns it into an errno.
#[derive(Debug)]
pub enum StreamError {
    NotFound,
    /// The item exists but the user may not do that (e.g. editing a Google
    /// Doc's shortcut, deleting someone else's file in a shared drive).
    NotPermitted,
    NotEmpty,
    IsDir,
    NotDir,
    Failed(AppError),
}

impl From<AppError> for StreamError {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
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
    /// New files are uploaded once unchanged for this long, not on every
    /// close: programs often close a file more than once while creating it,
    /// and save through a temporary file they rename over the original.
    pub new_file_quiet: Duration,
    /// After a failed upload (e.g. offline), wait this long before retrying.
    pub retry_backoff: Duration,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            listing_ttl: Duration::from_secs(30),
            block_size: 1 << 20,
            readahead_blocks: 8,
            new_file_quiet: Duration::from_secs(2),
            retry_backoff: Duration::from_secs(60),
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
    Shortcut {
        id: DriveItemId,
        bytes: Vec<u8>,
    },
    /// Content held in the staging area (being edited, or not uploaded yet).
    Staged(String),
    /// A folder that exists only on this computer: the desktop trash
    /// (`.Trash-<uid>` with `files/` and `info/`), see `trash_dir.rs`.
    Local(LocalDir),
    /// A desktop trash description (`info/*.trashinfo`), kept in memory.
    TrashInfo(Vec<u8>),
    /// Something moved to the desktop trash, i.e. to the Drive trash.
    Trashed {
        id: DriveItemId,
        dir: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalDir {
    Trash,
    TrashFiles,
    TrashInfo,
}

#[derive(Debug, Clone)]
struct Node {
    content: Content,
    modified: SystemTime,
}

/// A staged file, as this mount knows it.
#[derive(Debug, Clone)]
struct Staging {
    file: StagedFile,
    len: u64,
    /// Bumped by every write; the content is uploaded when it differs from
    /// `uploaded`, so writes during an upload are not lost.
    generation: u64,
    uploaded: u64,
    /// Last write, for `new_file_quiet`.
    changed: Instant,
    /// Set after a failed upload, for `retry_backoff`.
    retry_after: Option<Instant>,
}

impl Staging {
    fn dirty(&self) -> bool {
        self.generation != self.uploaded
    }
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
    staged: HashMap<String, Staging>,
    /// Drive files being edited → their staging key.
    staged_by_drive: HashMap<DriveItemId, String>,
    /// Children that exist only on this computer, per folder.
    local_children: HashMap<NodeId, Vec<(String, NodeId)>>,
    next: NodeId,
}

impl Table {
    fn attr(&self, id: NodeId) -> Option<NodeAttr> {
        let node = self.nodes.get(&id)?;
        let private = matches!(node.content, Content::Local(_));
        let (kind, size, modified) = match &node.content {
            Content::Root | Content::SharedDrives | Content::Folder(_) | Content::Local(_) => {
                (NodeKind::Dir, 0, node.modified)
            }
            Content::TrashInfo(bytes) => (NodeKind::File, bytes.len() as u64, node.modified),
            Content::Trashed { dir: true, .. } => (NodeKind::Dir, 0, node.modified),
            Content::Trashed { dir: false, .. } => (NodeKind::File, 0, node.modified),
            Content::File { size, .. } => (NodeKind::File, *size, node.modified),
            Content::Shortcut { bytes, .. } => (NodeKind::File, bytes.len() as u64, node.modified),
            Content::Staged(key) => {
                let staging = self.staged.get(key)?;
                (NodeKind::File, staging.len, staging.file.modified)
            }
        };
        Some(NodeAttr {
            id,
            kind,
            size,
            modified,
            private,
        })
    }

    fn content(&self, id: NodeId) -> Option<Content> {
        self.nodes.get(&id).map(|n| n.content.clone())
    }

    /// The node for `key` under `dir`, created if new.
    fn node_for(&mut self, dir: NodeId, key: String) -> NodeId {
        if let Some(id) = self.by_key.get(&(dir, key.clone())) {
            return *id;
        }
        let id = self.next;
        self.next += 1;
        self.by_key.insert((dir, key), id);
        id
    }

    /// Where the node of a staged file is, if it was listed.
    fn node_of_staged(&self, key: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(_, node)| matches!(&node.content, Content::Staged(k) if k == key))
            .map(|(id, _)| *id)
    }

    /// Forgets a folder listing, so the next request asks Drive again.
    fn invalidate(&mut self, dir: NodeId) {
        self.listings.remove(&dir);
    }
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
pub struct StreamingDrive<'a, A, C, D, K, S, G> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    cache: &'a K,
    settings: &'a S,
    staging: &'a G,
    state: &'a StreamState,
    account: &'a Account,
    /// Inserted in the names of conflict copies.
    conflict_label: String,
}

impl<'a, A, C, D, K, S, G> StreamingDrive<'a, A, C, D, K, S, G>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    K: BlockCache,
    S: SettingsRepository,
    G: StagingArea,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tokens: &'a TokenProvider<A, C>,
        drive: &'a D,
        cache: &'a K,
        settings: &'a S,
        staging: &'a G,
        state: &'a StreamState,
        account: &'a Account,
        conflict_label: String,
    ) -> Self {
        Self {
            tokens,
            drive,
            cache,
            settings,
            staging,
            state,
            account,
            conflict_label,
        }
    }

    /// `None` for ids this mount never handed out.
    pub fn attr(&self, id: NodeId) -> Option<NodeAttr> {
        self.state.table().attr(id)
    }

    pub async fn list(&self, dir: NodeId) -> Result<Vec<Entry>, StreamError> {
        let local_dir = matches!(self.state.table().content(dir), Some(Content::Local(_)));
        let fresh = if local_dir {
            Some(Vec::new())
        } else {
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
        let local = table.local_children.get(&dir).cloned().unwrap_or_default();
        Ok(children
            .into_iter()
            .chain(local)
            .filter_map(|(name, id)| {
                Some(Entry {
                    name,
                    attr: table.attr(id)?,
                })
            })
            .collect())
    }

    pub async fn lookup(&self, dir: NodeId, name: &str) -> Result<Option<Entry>, StreamError> {
        Ok(self.list(dir).await?.into_iter().find(|e| e.name == name))
    }

    /// Up to `size` bytes from `offset`, fetching only the blocks not cached yet.
    pub async fn read(&self, node: NodeId, offset: u64, size: u32) -> Result<Vec<u8>, StreamError> {
        let content = self.state.table().content(node);
        match content {
            Some(Content::Shortcut { bytes, .. }) => {
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                let end = start.saturating_add(size as usize).min(bytes.len());
                Ok(bytes[start..end].to_vec())
            }
            Some(Content::TrashInfo(bytes)) => {
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                let end = start.saturating_add(size as usize).min(bytes.len());
                Ok(bytes[start..end].to_vec())
            }
            // Its content is in the Drive trash.
            Some(Content::Trashed { .. }) => Ok(Vec::new()),
            Some(Content::Staged(key)) => Ok(self
                .staging
                .read_at(&key, offset, size as usize)
                .map_err(AppError::Storage)?),
            Some(Content::File {
                id,
                size: file_size,
                version,
            }) => {
                let config = self.state.config;
                let end = offset.saturating_add(u64::from(size)).min(file_size);
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
                Ok(self
                    .read_range(&id, &version, file_size, offset, end, ahead)
                    .await?)
            }
            Some(_) => Err(StreamError::IsDir),
            None => Err(StreamError::NotFound),
        }
    }

    /// Bytes `offset..end` of a Drive file, block by block.
    async fn read_range(
        &self,
        id: &DriveItemId,
        version: &str,
        file_size: u64,
        offset: u64,
        end: u64,
        ahead: u64,
    ) -> Result<Vec<u8>, AppError> {
        let end = end.min(file_size);
        if offset >= end {
            return Ok(Vec::new());
        }
        let block_size = self.state.config.block_size;
        let mut out = Vec::with_capacity(usize::try_from(end - offset).unwrap_or(0));
        for index in offset / block_size..=(end - 1) / block_size {
            let block = self.block(id, version, index, file_size, ahead).await?;
            let block_start = index * block_size;
            let from = usize::try_from(offset.max(block_start) - block_start).unwrap_or(0);
            let to = usize::try_from(end.min(block_start + block_size) - block_start).unwrap_or(0);
            out.extend_from_slice(&block[from.min(block.len())..to.min(block.len())]);
        }
        Ok(out)
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

    async fn fetch_listing(&self, dir: NodeId) -> Result<Vec<(String, NodeId)>, StreamError> {
        let content = self
            .state
            .table()
            .content(dir)
            .ok_or(StreamError::NotFound)?;
        let token = self.tokens.access_token(self.account).await?;
        // (key, display name, content, modified)
        let mut children: Vec<(String, String, Content, SystemTime)> = Vec::new();
        let mut folder_id = None;
        match content {
            Content::Root => {
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
            Content::SharedDrives => {
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
            Content::Folder(folder) => {
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
                folder_id = Some(folder);
            }
            _ => return Err(StreamError::NotDir),
        }

        let mut table = self.state.table();
        let mut listing = Vec::with_capacity(children.len());
        for (key, name, mut content, modified) in children {
            let id = table.node_for(dir, key);
            // A file being edited shows its staged content, not Drive's.
            if let Content::File { id: drive_id, .. } = &content
                && let Some(staged) = table.staged_by_drive.get(drive_id)
            {
                content = Content::Staged(staged.clone());
            }
            table.nodes.insert(id, Node { content, modified });
            listing.push((name, id));
        }
        // New files not uploaded yet (or never: editors' temporary files).
        if let Some(folder) = folder_id {
            let pending: Vec<(String, String)> = table
                .staged
                .values()
                .filter(
                    |s| matches!(&s.file.target, StagedTarget::New { parent } if *parent == folder),
                )
                .map(|s| (s.file.key.clone(), s.file.name.clone()))
                .collect();
            for (key, name) in pending {
                let id = table.node_for(dir, format!("staged:{key}"));
                table.nodes.insert(
                    id,
                    Node {
                        content: Content::Staged(key),
                        modified: self.state.mounted_at,
                    },
                );
                listing.push((name, id));
            }
        }
        table
            .listings
            .insert(dir, (Instant::now(), listing.clone()));
        Ok(listing)
    }
}

/// Drive's version of a file's content: its MD5, or else its modification time.
fn version_of(item: &DriveItem) -> String {
    match &item.kind {
        ItemKind::File { md5: Some(md5), .. } => md5.clone(),
        _ => {
            let nanos = item
                .modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("t{nanos}")
        }
    }
}

/// How a Drive item appears in the tree; `None` for items not shown.
fn content_of(item: &DriveItem) -> Option<Content> {
    match &item.kind {
        ItemKind::Folder => Some(Content::Folder(item.id.clone())),
        ItemKind::File { size, .. } => Some(Content::File {
            id: item.id.clone(),
            size: *size,
            version: version_of(item),
        }),
        ItemKind::GoogleDoc { url } => Some(Content::Shortcut {
            id: item.id.clone(),
            bytes: google_doc_shortcut(&item.name, url).into_bytes(),
        }),
        ItemKind::Shortcut | ItemKind::NotDownloadable => None,
    }
}

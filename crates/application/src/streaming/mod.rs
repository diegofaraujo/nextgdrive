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

use std::collections::{HashMap, VecDeque};
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
    /// How long a folder listing is trusted before asking Drive again. An
    /// older one is still shown at once, and refreshed in the background.
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
    /// An empty folder some trash implementations create in `files/` before
    /// renaming the trashed folder over it.
    Placeholder,
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
    listings: HashMap<NodeId, Listing>,
    /// Folders to list in the background (see `refresh_next`): stale ones
    /// shown first, then those listed ahead of being opened.
    refresh: VecDeque<NodeId>,
    /// Bumped whenever listings are forgotten, so a background listing that
    /// started before a change here is not stored over it.
    epoch: u64,
    /// Where the last read of each file ended, to recognize sequential reads.
    read_ends: HashMap<NodeId, u64>,
    staged: HashMap<String, Staging>,
    /// Drive files being edited → their staging key.
    staged_by_drive: HashMap<DriveItemId, String>,
    /// Children that exist only on this computer, per folder.
    local_children: HashMap<NodeId, Vec<(String, NodeId)>>,
    next: NodeId,
}

/// Children of a listed folder, with their names.
#[derive(Debug, Clone)]
struct Listing {
    at: Instant,
    children: Vec<(String, NodeId)>,
    /// Whether its subfolders were queued to be listed ahead.
    expanded: bool,
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
        self.epoch += 1;
    }

    fn invalidate_all(&mut self) {
        self.listings.clear();
        self.epoch += 1;
    }

    /// Queues `dir` to be listed in the background; `urgent` ones go first.
    fn queue_refresh(&mut self, dir: NodeId, urgent: bool) {
        let queued = self.refresh.iter().position(|d| *d == dir);
        match (queued, urgent) {
            (Some(at), true) => {
                self.refresh.remove(at);
                self.refresh.push_front(dir);
            }
            (Some(_), false) => {}
            (None, true) => self.refresh.push_front(dir),
            (None, false) => self.refresh.push_back(dir),
        }
    }

    /// A folder's listing, even if stale (then queued to be refreshed). The
    /// first time it is shown, its subfolders not listed yet are queued, so
    /// they open at once.
    fn shown_listing(&mut self, dir: NodeId, ttl: Duration) -> Option<Vec<(String, NodeId)>> {
        let listing = self.listings.get_mut(&dir)?;
        let stale = listing.at.elapsed() >= ttl;
        let expand = !std::mem::replace(&mut listing.expanded, true);
        let children = listing.children.clone();
        if stale {
            self.queue_refresh(dir, true);
        }
        if expand {
            for (_, id) in &children {
                let folder = self.nodes.get(id).is_some_and(|n| {
                    matches!(n.content, Content::Folder(_) | Content::SharedDrives)
                });
                if folder && !self.listings.contains_key(id) {
                    self.queue_refresh(*id, false);
                }
            }
        }
        Some(children)
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
        let children = if local_dir {
            Vec::new()
        } else {
            let ttl = self.state.config.listing_ttl;
            let shown = self.state.table().shown_listing(dir, ttl);
            match shown {
                Some(children) => children,
                None => {
                    let children = self.fetch_children(dir).await?;
                    let mut table = self.state.table();
                    self.store_listing(&mut table, dir, children);
                    table.shown_listing(dir, ttl).unwrap_or_default()
                }
            }
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

    /// Lists every folder from the account folder down to the item `target`,
    /// and every folder inside it, so all of it can be browsed offline.
    pub async fn preload(&self, target: &DriveItemId) -> Result<(), StreamError> {
        let token = self.tokens.access_token(self.account).await?;
        // The chain of folders from the drive's root down to `target`.
        let drives = self.drive.drives(&token).await.map_err(AppError::Drive)?;
        // For a file, the folder holding it.
        let item = self
            .drive
            .item(&token, target)
            .await
            .map_err(AppError::Drive)?;
        let target = match item.kind {
            ItemKind::Folder => item.id,
            _ => item.parents.first().cloned().ok_or(StreamError::NotFound)?,
        };
        let mut chain = vec![target];
        loop {
            let top = chain.last().expect("never empty").clone();
            if drives.iter().any(|d| d.root == top) {
                break;
            }
            let item = self
                .drive
                .item(&token, &top)
                .await
                .map_err(AppError::Drive)?;
            match item.parents.first() {
                Some(parent) if chain.len() < 256 => chain.push(parent.clone()),
                _ => return Err(StreamError::NotFound),
            }
        }
        chain.reverse();
        // Walk down from the account folder, through "Shared drives" if needed.
        let mut node = ROOT;
        let shared = drives
            .iter()
            .any(|d| d.root == chain[0] && matches!(d.kind, DriveKind::Shared { .. }));
        if shared {
            node = self
                .child(node, |c| matches!(c, Content::SharedDrives))
                .await?;
        }
        for id in &chain {
            node = self
                .child(node, |c| matches!(c, Content::Folder(f) if f == id))
                .await?;
        }
        // Then everything inside.
        let mut pending = vec![node];
        while let Some(dir) = pending.pop() {
            for entry in self.list(dir).await? {
                if entry.attr.kind == NodeKind::Dir {
                    pending.push(entry.attr.id);
                }
            }
        }
        Ok(())
    }

    /// The child of `dir` whose content matches.
    async fn child(
        &self,
        dir: NodeId,
        matches: impl Fn(&Content) -> bool,
    ) -> Result<NodeId, StreamError> {
        let entries = self.list(dir).await?;
        let table = self.state.table();
        entries
            .iter()
            .map(|e| e.attr.id)
            .find(|id| table.nodes.get(id).is_some_and(|n| matches(&n.content)))
            .ok_or(StreamError::NotFound)
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

    /// Lists the next folder waiting to be listed in the background, if any
    /// (`false` once none is left). One that became fresh meanwhile is
    /// skipped; if listing fails (e.g. offline), the stale listing is kept
    /// and trusted for another `listing_ttl` before trying again.
    pub async fn refresh_next(&self) -> bool {
        let (dir, epoch) = {
            let mut table = self.state.table();
            let Some(dir) = table.refresh.pop_front() else {
                return false;
            };
            let ttl = self.state.config.listing_ttl;
            let fresh = table
                .listings
                .get(&dir)
                .is_some_and(|l| l.at.elapsed() < ttl);
            if fresh {
                return true;
            }
            (dir, table.epoch)
        };
        let fetched = self.fetch_children(dir).await;
        let mut table = self.state.table();
        match fetched {
            Ok(children) if table.epoch == epoch => self.store_listing(&mut table, dir, children),
            // Something changed here meanwhile; the next request lists it again.
            Ok(_) => {}
            Err(_) => {
                if let Some(listing) = table.listings.get_mut(&dir) {
                    listing.at = Instant::now();
                }
            }
        }
        true
    }

    /// Whether folders are waiting to be listed in the background.
    pub fn refresh_pending(&self) -> bool {
        !self.state.table().refresh.is_empty()
    }

    /// A folder's children in Drive: (key, display name, content, modified).
    async fn fetch_children(&self, dir: NodeId) -> Result<Vec<Child>, StreamError> {
        let content = self
            .state
            .table()
            .content(dir)
            .ok_or(StreamError::NotFound)?;
        let token = self.tokens.access_token(self.account).await?;
        let mut children: Vec<Child> = Vec::new();
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
            }
            _ => return Err(StreamError::NotDir),
        }
        Ok(children)
    }

    /// Records `dir`'s children, keeping the nodes of those listed before.
    fn store_listing(&self, table: &mut Table, dir: NodeId, children: Vec<Child>) {
        let folder_id = match table.content(dir) {
            Some(Content::Folder(folder)) => Some(folder),
            _ => None,
        };
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
        table.listings.insert(
            dir,
            Listing {
                at: Instant::now(),
                children: listing,
                expanded: false,
            },
        );
    }
}

/// A folder's child as Drive lists it: (key, display name, content, modified).
type Child = (String, String, Content, SystemTime);

/// Drive's version of a file's content: its MD5, or else its modification time.
pub(crate) fn version_of(item: &DriveItem) -> String {
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

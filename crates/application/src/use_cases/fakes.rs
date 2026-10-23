use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};

use nextgdrive_domain::{
    Account, AccountId, Capabilities, Drive, DriveItem, DriveItemId, DriveKind, Email, ItemKind,
    Settings, StorageQuota, TrashedItem,
};

use crate::ports::{
    AccessGrant, AccessToken, AccountRepository, Authenticator, AuthorizedTokens, Changes,
    CredentialStore, DirEntry, DriveAccountInfo, DriveService, FileReader, FileWriter,
    LocalFileSystem, LocalMetadata, PendingAuthorization, PortError, RefreshToken,
    SettingsRepository, SyncStateStore, SyncedItem, UploadTarget,
};

#[derive(Default)]
pub struct InMemoryAccounts(Mutex<Vec<Account>>);

impl InMemoryAccounts {
    pub fn with(accounts: impl IntoIterator<Item = Account>) -> Self {
        Self(Mutex::new(accounts.into_iter().collect()))
    }
}

impl AccountRepository for InMemoryAccounts {
    fn list(&self) -> Result<Vec<Account>, PortError> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn find_by_email(&self, email: &Email) -> Result<Option<Account>, PortError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|a| &a.email == email)
            .cloned())
    }

    fn save(&self, account: &Account) -> Result<(), PortError> {
        let mut accounts = self.0.lock().unwrap();
        match accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account.clone(),
            None => accounts.push(account.clone()),
        }
        Ok(())
    }

    fn remove(&self, id: &AccountId) -> Result<(), PortError> {
        self.0.lock().unwrap().retain(|a| &a.id != id);
        Ok(())
    }
}

pub struct InMemorySettings(Mutex<Settings>);

impl Default for InMemorySettings {
    fn default() -> Self {
        Self(Mutex::new(Settings {
            sync_root: "/home/ana/GoogleDrive".into(),
            my_drive_folder: "My Drive".into(),
            shared_drives_folder: "Shared drives".into(),
            poll_interval_secs: Settings::DEFAULT_POLL_INTERVAL_SECS,
            notifications: true,
            selections: BTreeMap::new(),
        }))
    }
}

impl InMemorySettings {
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.0.lock().unwrap());
    }
}

impl SettingsRepository for InMemorySettings {
    fn load(&self) -> Result<Settings, PortError> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, settings: &Settings) -> Result<(), PortError> {
        *self.0.lock().unwrap() = settings.clone();
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryCredentials(Mutex<HashMap<AccountId, RefreshToken>>);

impl InMemoryCredentials {
    pub fn with_token(id: &str, token: &str) -> Self {
        Self(Mutex::new(HashMap::from([(
            AccountId::new(id),
            RefreshToken::new(token),
        )])))
    }

    pub fn token(&self, id: &str) -> Option<RefreshToken> {
        self.0.lock().unwrap().get(&AccountId::new(id)).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }
}

impl CredentialStore for InMemoryCredentials {
    async fn store(&self, account: &AccountId, token: &RefreshToken) -> Result<(), PortError> {
        self.0
            .lock()
            .unwrap()
            .insert(account.clone(), token.clone());
        Ok(())
    }

    async fn load(&self, account: &AccountId) -> Result<Option<RefreshToken>, PortError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }

    async fn delete(&self, account: &AccountId) -> Result<(), PortError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

/// Signs in immediately, always with the same tokens. Counts refreshes.
#[derive(Default)]
pub struct FakeAuth {
    refreshes: AtomicUsize,
}

pub struct FakePending;

impl FakeAuth {
    pub const URL: &str = "https://accounts.example/sign-in";
    pub const REFRESH_TOKEN: &str = "refresh";
    pub const ACCESS_TOKEN: &str = "access";

    pub fn refresh_count(&self) -> usize {
        self.refreshes.load(Ordering::Relaxed)
    }
}

impl PendingAuthorization for FakePending {
    fn url(&self) -> &str {
        FakeAuth::URL
    }
}

impl Authenticator for FakeAuth {
    type Pending = FakePending;

    async fn start(&self) -> Result<FakePending, PortError> {
        Ok(FakePending)
    }

    async fn finish(&self, _: FakePending) -> Result<AuthorizedTokens, PortError> {
        Ok(AuthorizedTokens {
            access: AccessToken::new(Self::ACCESS_TOKEN),
            refresh: RefreshToken::new(Self::REFRESH_TOKEN),
        })
    }

    async fn refresh(&self, token: &RefreshToken) -> Result<AccessGrant, PortError> {
        self.refreshes.fetch_add(1, Ordering::Relaxed);
        if token.secret() == Self::REFRESH_TOKEN {
            Ok(AccessGrant {
                token: AccessToken::new(Self::ACCESS_TOKEN),
                expires_in: Duration::from_secs(3600),
            })
        } else {
            Err(PortError::new("unknown refresh token"))
        }
    }
}

/// A Drive with one account, a My Drive with root id `root`, and items added
/// with the `with_*` builders. Only accepts [`FakeAuth::ACCESS_TOKEN`].
pub struct FakeDrive {
    account: Account,
    inner: Mutex<DriveState>,
    pub changes_calls: AtomicUsize,
}

#[derive(Default)]
struct DriveState {
    items: BTreeMap<DriveItemId, DriveItem>,
    contents: BTreeMap<DriveItemId, Vec<u8>>,
    /// Folder whose listing fails, to simulate API errors.
    failing: Option<DriveItemId>,
    /// File whose download returns content that does not match its MD5.
    corrupt: Option<DriveItemId>,
    clock: u64,
    /// Bumped by every change, like Drive's change feed position.
    version: u64,
    next_id: u64,
    trashed: Vec<DriveItemId>,
    /// Trashed items, by the item the user trashed: everything hidden with it.
    bin: BTreeMap<DriveItemId, (SystemTime, Vec<DriveItem>)>,
    removed_from_my_drive: Vec<DriveItemId>,
}

impl DriveState {
    fn tick(&mut self) -> SystemTime {
        self.clock += 1;
        self.version += 1;
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.clock)
    }

    fn item_mut(&mut self, id: &str) -> &mut DriveItem {
        self.version += 1;
        self.items
            .get_mut(&DriveItemId::new(id))
            .expect("no such item")
    }

    fn new_id(&mut self) -> DriveItemId {
        self.next_id += 1;
        DriveItemId::new(format!("new-{}", self.next_id))
    }

    /// Hides an item and everything inside it, as the trash does, and keeps
    /// them in the bin.
    fn hide(&mut self, id: &DriveItemId) {
        let mut hidden = Vec::new();
        self.collect(id, &mut hidden);
        let at = self.tick();
        self.bin.insert(id.clone(), (at, hidden));
    }

    fn collect(&mut self, id: &DriveItemId, hidden: &mut Vec<DriveItem>) {
        let children: Vec<DriveItemId> = self
            .items
            .values()
            .filter(|i| i.parents.contains(id))
            .map(|i| i.id.clone())
            .collect();
        for child in children {
            self.collect(&child, hidden);
        }
        hidden.extend(self.items.remove(id));
    }
}

pub fn md5_hex(content: &[u8]) -> String {
    Md5::digest(content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn file_kind(content: &[u8]) -> ItemKind {
    ItemKind::File {
        size: content.len() as u64,
        md5: Some(md5_hex(content)),
    }
}

impl FakeDrive {
    pub const QUOTA: StorageQuota = StorageQuota {
        usage: 5,
        limit: Some(15),
    };

    pub fn new(account: Account) -> Self {
        let root = DriveItem {
            id: DriveItemId::new("root"),
            name: "My Drive".into(),
            kind: ItemKind::Folder,
            modified: SystemTime::UNIX_EPOCH,
            parents: vec![],
            capabilities: Capabilities::default(),
        };
        let mut state = DriveState::default();
        state.items.insert(root.id.clone(), root);
        Self {
            account,
            inner: Mutex::new(state),
            changes_calls: AtomicUsize::new(0),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, DriveState> {
        self.inner.lock().unwrap()
    }

    pub fn with_item(self, parent: &str, id: &str, name: &str, kind: ItemKind) -> Self {
        {
            let mut state = self.state();
            let modified = state.tick();
            let item = DriveItem {
                id: DriveItemId::new(id),
                name: name.into(),
                kind,
                modified,
                parents: vec![DriveItemId::new(parent)],
                capabilities: Capabilities::default(),
            };
            state.items.insert(item.id.clone(), item);
        }
        self
    }

    pub fn with_folder(self, parent: &str, id: &str, name: &str) -> Self {
        self.with_item(parent, id, name, ItemKind::Folder)
    }

    pub fn with_file(self, parent: &str, id: &str, name: &str) -> Self {
        self.with_content(parent, id, name, name)
    }

    pub fn with_content(self, parent: &str, id: &str, name: &str, content: &str) -> Self {
        let drive = self.with_item(parent, id, name, file_kind(content.as_bytes()));
        drive
            .state()
            .contents
            .insert(DriveItemId::new(id), content.as_bytes().to_vec());
        drive
    }

    pub fn set_content(&self, id: &str, content: &str) {
        let mut state = self.state();
        let modified = state.tick();
        let item = state.item_mut(id);
        item.kind = file_kind(content.as_bytes());
        item.modified = modified;
        let id = item.id.clone();
        state.contents.insert(id, content.as_bytes().to_vec());
    }

    pub fn rename(&self, id: &str, name: &str) {
        self.state().item_mut(id).name = name.into();
    }

    pub fn move_to(&self, id: &str, parent: &str) {
        self.state().item_mut(id).parents = vec![DriveItemId::new(parent)];
    }

    pub fn remove(&self, id: &str) {
        let mut state = self.state();
        state.version += 1;
        state.hide(&DriveItemId::new(id));
    }

    pub fn set_capabilities(&self, id: &str, capabilities: Capabilities) {
        self.state().item_mut(id).capabilities = capabilities;
    }

    pub fn fail_listing(&self, folder: &str) {
        self.state().failing = Some(DriveItemId::new(folder));
    }

    pub fn corrupt_download(&self, id: &str) {
        self.state().corrupt = Some(DriveItemId::new(id));
    }

    pub fn content(&self, id: &str) -> Option<String> {
        let state = self.state();
        let bytes = state.contents.get(&DriveItemId::new(id))?;
        Some(String::from_utf8(bytes.clone()).unwrap())
    }

    /// The item named `name` directly inside `parent`, if any.
    pub fn find(&self, parent: &str, name: &str) -> Option<DriveItem> {
        let parent = DriveItemId::new(parent);
        self.state()
            .items
            .values()
            .find(|i| i.parents.contains(&parent) && i.name == name)
            .cloned()
    }

    pub fn has(&self, id: &str) -> bool {
        self.state().items.contains_key(&DriveItemId::new(id))
    }

    pub fn trashed(&self) -> Vec<String> {
        self.state()
            .trashed
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    pub fn removed_from_my_drive(&self) -> Vec<String> {
        self.state()
            .removed_from_my_drive
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn check(token: &AccessToken) -> Result<(), PortError> {
        if token.secret() == FakeAuth::ACCESS_TOKEN {
            Ok(())
        } else {
            Err(PortError::new("invalid access token"))
        }
    }

    fn get(&self, id: &DriveItemId) -> Result<DriveItem, PortError> {
        self.state()
            .items
            .get(id)
            .cloned()
            .ok_or_else(|| PortError::new(format!("no item {id}")))
    }
}

impl DriveService for FakeDrive {
    async fn account_info(&self, token: &AccessToken) -> Result<DriveAccountInfo, PortError> {
        Self::check(token)?;
        Ok(DriveAccountInfo {
            account: self.account.clone(),
            quota: Self::QUOTA,
        })
    }

    async fn drives(&self, token: &AccessToken) -> Result<Vec<Drive>, PortError> {
        Self::check(token)?;
        Ok(vec![Drive {
            root: DriveItemId::new("root"),
            kind: DriveKind::MyDrive,
        }])
    }

    async fn children(
        &self,
        token: &AccessToken,
        folder: &DriveItemId,
    ) -> Result<Vec<DriveItem>, PortError> {
        Self::check(token)?;
        let state = self.state();
        if state.failing.as_ref() == Some(folder) {
            return Err(PortError::new("listing failed"));
        }
        Ok(state
            .items
            .values()
            .filter(|item| item.parents.contains(folder))
            .cloned()
            .collect())
    }

    async fn item(&self, token: &AccessToken, id: &DriveItemId) -> Result<DriveItem, PortError> {
        Self::check(token)?;
        self.get(id)
    }

    async fn changes_since(
        &self,
        token: &AccessToken,
        cursor: Option<&str>,
    ) -> Result<Changes, PortError> {
        Self::check(token)?;
        self.changes_calls.fetch_add(1, Ordering::Relaxed);
        let current = self.state().version.to_string();
        Ok(Changes {
            any: cursor != Some(current.as_str()),
            cursor: current,
        })
    }

    async fn download<W: FileWriter>(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        out: &mut W,
    ) -> Result<(), PortError> {
        Self::check(token)?;
        let content = {
            let state = self.state();
            if state.corrupt.as_ref() == Some(id) {
                b"corrupted".to_vec()
            } else {
                state
                    .contents
                    .get(id)
                    .cloned()
                    .ok_or_else(|| PortError::new("no content"))?
            }
        };
        // Several chunks, like a real download.
        for chunk in content.chunks(3) {
            out.write(chunk).await?;
        }
        Ok(())
    }

    async fn create_folder(
        &self,
        token: &AccessToken,
        parent: &DriveItemId,
        name: &str,
    ) -> Result<DriveItem, PortError> {
        Self::check(token)?;
        let mut state = self.state();
        let modified = state.tick();
        let item = DriveItem {
            id: state.new_id(),
            name: name.into(),
            kind: ItemKind::Folder,
            modified,
            parents: vec![parent.clone()],
            capabilities: Capabilities::default(),
        };
        state.items.insert(item.id.clone(), item.clone());
        Ok(item)
    }

    async fn upload<R: FileReader>(
        &self,
        token: &AccessToken,
        target: UploadTarget<'_>,
        size: u64,
        modified: SystemTime,
        content: &mut R,
    ) -> Result<DriveItem, PortError> {
        Self::check(token)?;
        let mut bytes = Vec::new();
        let mut buf = [0; 4];
        loop {
            let read = content.read(&mut buf).await?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..read]);
        }
        if bytes.len() as u64 != size {
            return Err(PortError::new("size mismatch"));
        }
        let mut state = self.state();
        state.version += 1;
        let item = match target {
            UploadTarget::New { parent, name } => {
                let item = DriveItem {
                    id: state.new_id(),
                    name: name.into(),
                    kind: file_kind(&bytes),
                    modified,
                    parents: vec![parent.clone()],
                    capabilities: Capabilities::default(),
                };
                state.items.insert(item.id.clone(), item.clone());
                item
            }
            UploadTarget::Existing(id) => {
                let item = state
                    .items
                    .get_mut(id)
                    .ok_or_else(|| PortError::new("no such file"))?;
                if !item.capabilities.edit {
                    return Err(PortError::new("read-only"));
                }
                item.kind = file_kind(&bytes);
                item.modified = modified;
                item.clone()
            }
        };
        state.contents.insert(item.id.clone(), bytes);
        Ok(item)
    }

    async fn move_item(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        from: &DriveItemId,
        to: &DriveItemId,
        name: &str,
    ) -> Result<DriveItem, PortError> {
        Self::check(token)?;
        let mut state = self.state();
        state.version += 1;
        let item = state
            .items
            .get_mut(id)
            .ok_or_else(|| PortError::new("no such item"))?;
        if !item.parents.contains(from) {
            return Err(PortError::new("not in that folder"));
        }
        item.parents = vec![to.clone()];
        item.name = name.into();
        Ok(item.clone())
    }

    async fn trash(&self, token: &AccessToken, id: &DriveItemId) -> Result<(), PortError> {
        Self::check(token)?;
        let mut state = self.state();
        if !self.get_unlocked(&state, id)?.capabilities.trash {
            return Err(PortError::new("not allowed"));
        }
        state.version += 1;
        state.hide(id);
        state.trashed.push(id.clone());
        Ok(())
    }

    async fn trashed_items(&self, token: &AccessToken) -> Result<Vec<TrashedItem>, PortError> {
        Self::check(token)?;
        let state = self.state();
        Ok(state
            .bin
            .iter()
            .filter_map(|(id, (at, hidden))| {
                let item = hidden.iter().find(|i| &i.id == id)?.clone();
                Some(TrashedItem {
                    item,
                    trashed_at: Some(*at),
                })
            })
            .collect())
    }

    async fn restore(&self, token: &AccessToken, id: &DriveItemId) -> Result<DriveItem, PortError> {
        Self::check(token)?;
        let mut state = self.state();
        let (_, hidden) = state
            .bin
            .remove(id)
            .ok_or_else(|| PortError::new("not in the trash"))?;
        state.version += 1;
        let restored = hidden.iter().find(|i| &i.id == id).cloned();
        for item in hidden {
            state.items.insert(item.id.clone(), item);
        }
        restored.ok_or_else(|| PortError::new("not in the trash"))
    }

    async fn remove_from_my_drive(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        parent: &DriveItemId,
    ) -> Result<(), PortError> {
        Self::check(token)?;
        let mut state = self.state();
        state.version += 1;
        let item = state
            .items
            .get_mut(id)
            .ok_or_else(|| PortError::new("no such item"))?;
        item.parents.retain(|p| p != parent);
        state.removed_from_my_drive.push(id.clone());
        Ok(())
    }
}

impl FakeDrive {
    fn get_unlocked<'s>(
        &self,
        state: &'s DriveState,
        id: &DriveItemId,
    ) -> Result<&'s DriveItem, PortError> {
        state
            .items
            .get(id)
            .ok_or_else(|| PortError::new(format!("no item {id}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Dir {
        inode: u64,
    },
    File {
        content: Vec<u8>,
        modified: SystemTime,
        inode: u64,
    },
}

/// In-memory filesystem. Paths must be absolute. Like a real one, renames
/// keep inodes and every write (save) creates a new one.
#[derive(Default)]
pub struct FakeFileSystem {
    nodes: Mutex<BTreeMap<PathBuf, Node>>,
    clock: AtomicU64,
    inodes: AtomicU64,
}

pub struct FakeWriter {
    path: PathBuf,
    content: Vec<u8>,
}

impl FileWriter for FakeWriter {
    async fn write(&mut self, chunk: &[u8]) -> Result<(), PortError> {
        self.content.extend_from_slice(chunk);
        Ok(())
    }

    fn md5_hex(&self) -> String {
        md5_hex(&self.content)
    }
}

pub struct FakeReader {
    content: Vec<u8>,
    position: usize,
}

impl FileReader for FakeReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, PortError> {
        let rest = &self.content[self.position..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.position += n;
        Ok(n)
    }
}

impl FakeFileSystem {
    fn inode(&self) -> u64 {
        self.inodes.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Simulates the user saving a file, with a new modification time.
    pub fn write_local(&self, path: &str, content: &str) {
        let modified = SystemTime::UNIX_EPOCH
            + Duration::from_secs(1_000_000 + self.clock.fetch_add(1, Ordering::Relaxed));
        let path = PathBuf::from(path);
        self.create_dir_all(path.parent().unwrap()).unwrap();
        let inode = self.inode();
        self.nodes.lock().unwrap().insert(
            path,
            Node::File {
                content: content.as_bytes().to_vec(),
                modified,
                inode,
            },
        );
    }

    /// Simulates the user deleting a file or a whole folder.
    pub fn delete_local(&self, path: &str) {
        self.nodes
            .lock()
            .unwrap()
            .retain(|p, _| !p.starts_with(path));
    }

    /// Simulates the user moving or renaming something.
    pub fn move_local(&self, from: &str, to: &str) {
        let to = PathBuf::from(to);
        self.create_dir_all(to.parent().unwrap()).unwrap();
        self.rename(Path::new(from), &to).unwrap();
    }

    pub fn read(&self, path: &str) -> Option<String> {
        match self.nodes.lock().unwrap().get(Path::new(path)) {
            Some(Node::File { content, .. }) => Some(String::from_utf8(content.clone()).unwrap()),
            _ => None,
        }
    }

    pub fn is_dir(&self, path: &str) -> bool {
        matches!(
            self.nodes.lock().unwrap().get(Path::new(path)),
            Some(Node::Dir { .. })
        )
    }

    /// Every file, relative to `base`.
    pub fn files_under(&self, base: &str) -> Vec<String> {
        self.nodes
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, node)| matches!(node, Node::File { .. }))
            .filter_map(|(path, _)| path.strip_prefix(base).ok())
            .map(|path| path.display().to_string())
            .collect()
    }
}

fn meta(node: &Node) -> LocalMetadata {
    match node {
        Node::Dir { inode } => LocalMetadata {
            is_dir: true,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            inode: *inode,
        },
        Node::File {
            content,
            modified,
            inode,
        } => LocalMetadata {
            is_dir: false,
            size: content.len() as u64,
            modified: *modified,
            inode: *inode,
        },
    }
}

impl LocalFileSystem for FakeFileSystem {
    type Writer = FakeWriter;
    type Reader = FakeReader;

    fn metadata(&self, path: &Path) -> Result<Option<LocalMetadata>, PortError> {
        Ok(self.nodes.lock().unwrap().get(path).map(meta))
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, PortError> {
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.parent() == Some(path))
            .map(|(p, node)| DirEntry {
                name: p.file_name().unwrap().to_string_lossy().into_owned(),
                metadata: meta(node),
            })
            .collect())
    }

    async fn open_read(&self, path: &Path) -> Result<FakeReader, PortError> {
        match self.nodes.lock().unwrap().get(path) {
            Some(Node::File { content, .. }) => Ok(FakeReader {
                content: content.clone(),
                position: 0,
            }),
            _ => Err(PortError::new("not a file")),
        }
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), PortError> {
        let missing: Vec<PathBuf> = {
            let nodes = self.nodes.lock().unwrap();
            let mut missing = Vec::new();
            for dir in path.ancestors() {
                match nodes.get(dir) {
                    Some(Node::File { .. }) => {
                        return Err(PortError::new(format!("{} is a file", dir.display())));
                    }
                    Some(Node::Dir { .. }) => {}
                    None => missing.push(dir.to_owned()),
                }
            }
            missing
        };
        for dir in missing {
            let inode = self.inode();
            self.nodes.lock().unwrap().insert(dir, Node::Dir { inode });
        }
        Ok(())
    }

    async fn begin_write(&self, path: &Path) -> Result<FakeWriter, PortError> {
        match self.nodes.lock().unwrap().get(path.parent().unwrap()) {
            Some(Node::Dir { .. }) => Ok(FakeWriter {
                path: path.to_owned(),
                content: Vec::new(),
            }),
            _ => Err(PortError::new("parent is not a directory")),
        }
    }

    async fn commit(
        &self,
        writer: FakeWriter,
        modified: SystemTime,
    ) -> Result<LocalMetadata, PortError> {
        let node = Node::File {
            content: writer.content,
            modified,
            inode: self.inode(),
        };
        let metadata = meta(&node);
        self.nodes.lock().unwrap().insert(writer.path, node);
        Ok(metadata)
    }

    async fn md5_hex(&self, path: &Path) -> Result<String, PortError> {
        match self.nodes.lock().unwrap().get(path) {
            Some(Node::File { content, .. }) => Ok(md5_hex(content)),
            _ => Err(PortError::new("not a file")),
        }
    }

    async fn has_content(&self, path: &Path, expected: &[u8]) -> Result<bool, PortError> {
        Ok(matches!(
            self.nodes.lock().unwrap().get(path),
            Some(Node::File { content, .. }) if content == expected
        ))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), PortError> {
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.contains_key(to) {
            return Err(PortError::new("target exists"));
        }
        let moved: Vec<PathBuf> = nodes
            .keys()
            .filter(|p| p.starts_with(from))
            .cloned()
            .collect();
        if moved.is_empty() {
            return Err(PortError::new("source missing"));
        }
        for old in moved {
            let node = nodes.remove(&old).unwrap();
            nodes.insert(to.join(old.strip_prefix(from).unwrap()), node);
        }
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<(), PortError> {
        match self.nodes.lock().unwrap().remove(path) {
            Some(Node::File { .. }) => Ok(()),
            _ => Err(PortError::new("not a file")),
        }
    }

    fn remove_dir_if_empty(&self, path: &Path) -> Result<bool, PortError> {
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.keys().any(|p| p != path && p.starts_with(path)) {
            return Ok(false);
        }
        Ok(nodes.remove(path).is_some())
    }
}

#[derive(Default)]
pub struct InMemorySyncState {
    items: Mutex<HashMap<(AccountId, DriveItemId), SyncedItem>>,
    cursors: Mutex<HashMap<AccountId, String>>,
    locked: Arc<Mutex<HashSet<AccountId>>>,
}

pub struct FakeLock(Arc<Mutex<HashSet<AccountId>>>, AccountId);

impl Drop for FakeLock {
    fn drop(&mut self) {
        self.0.lock().unwrap().remove(&self.1);
    }
}

impl SyncStateStore for InMemorySyncState {
    type Lock = FakeLock;

    fn try_lock(&self, account: &AccountId) -> Result<Option<FakeLock>, PortError> {
        let inserted = self.locked.lock().unwrap().insert(account.clone());
        Ok(inserted.then(|| FakeLock(self.locked.clone(), account.clone())))
    }

    fn cursor(&self, account: &AccountId) -> Result<Option<String>, PortError> {
        Ok(self.cursors.lock().unwrap().get(account).cloned())
    }

    fn set_cursor(&self, account: &AccountId, cursor: &str) -> Result<(), PortError> {
        self.cursors
            .lock()
            .unwrap()
            .insert(account.clone(), cursor.to_owned());
        Ok(())
    }

    fn items(&self, account: &AccountId) -> Result<Vec<SyncedItem>, PortError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .filter(|((a, _), _)| a == account)
            .map(|(_, item)| item.clone())
            .collect())
    }

    fn upsert(&self, account: &AccountId, item: &SyncedItem) -> Result<(), PortError> {
        self.items
            .lock()
            .unwrap()
            .insert((account.clone(), item.id.clone()), item.clone());
        Ok(())
    }

    fn remove(&self, account: &AccountId, item: &DriveItemId) -> Result<(), PortError> {
        self.items
            .lock()
            .unwrap()
            .remove(&(account.clone(), item.clone()));
        Ok(())
    }
}

pub fn account(id: &str, email: &str) -> Account {
    Account {
        id: AccountId::new(id),
        email: Email::parse(email).unwrap(),
        display_name: None,
    }
}

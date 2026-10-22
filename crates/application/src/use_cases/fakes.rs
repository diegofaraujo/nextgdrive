use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};

use nextgdrive_domain::{
    Account, AccountId, Drive, DriveItem, DriveItemId, DriveKind, Email, ItemKind, Settings,
    StorageQuota,
};

use crate::ports::{
    AccessGrant, AccessToken, AccountRepository, Authenticator, AuthorizedTokens, CredentialStore,
    DriveAccountInfo, DriveService, FileWriter, LocalFileSystem, LocalMetadata,
    PendingAuthorization, PortError, RefreshToken, SettingsRepository, SyncStateStore, SyncedItem,
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
    items: BTreeMap<DriveItemId, DriveItem>,
    contents: BTreeMap<DriveItemId, Vec<u8>>,
    /// Folder whose listing fails, to simulate API errors.
    failing: Option<DriveItemId>,
    /// File whose download returns content that does not match its MD5.
    corrupt: Option<DriveItemId>,
    clock: u64,
}

pub fn md5_hex(content: &[u8]) -> String {
    Md5::digest(content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
        };
        Self {
            account,
            items: BTreeMap::from([(root.id.clone(), root)]),
            contents: BTreeMap::new(),
            failing: None,
            corrupt: None,
            clock: 0,
        }
    }

    fn tick(&mut self) -> SystemTime {
        self.clock += 1;
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.clock)
    }

    pub fn with_item(mut self, parent: &str, id: &str, name: &str, kind: ItemKind) -> Self {
        let item = DriveItem {
            id: DriveItemId::new(id),
            name: name.into(),
            kind,
            modified: self.tick(),
            parents: vec![DriveItemId::new(parent)],
        };
        self.items.insert(item.id.clone(), item);
        self
    }

    pub fn with_folder(self, parent: &str, id: &str, name: &str) -> Self {
        self.with_item(parent, id, name, ItemKind::Folder)
    }

    pub fn with_file(self, parent: &str, id: &str, name: &str) -> Self {
        self.with_content(parent, id, name, name)
    }

    pub fn with_content(self, parent: &str, id: &str, name: &str, content: &str) -> Self {
        let kind = ItemKind::File {
            size: content.len() as u64,
            md5: Some(md5_hex(content.as_bytes())),
        };
        let mut drive = self.with_item(parent, id, name, kind);
        drive
            .contents
            .insert(DriveItemId::new(id), content.as_bytes().to_vec());
        drive
    }

    pub fn set_content(&mut self, id: &str, content: &str) {
        let modified = self.tick();
        let item = self.items.get_mut(&DriveItemId::new(id)).unwrap();
        item.kind = ItemKind::File {
            size: content.len() as u64,
            md5: Some(md5_hex(content.as_bytes())),
        };
        item.modified = modified;
        self.contents
            .insert(item.id.clone(), content.as_bytes().to_vec());
    }

    pub fn rename(&mut self, id: &str, name: &str) {
        self.items.get_mut(&DriveItemId::new(id)).unwrap().name = name.into();
    }

    pub fn move_to(&mut self, id: &str, parent: &str) {
        self.items.get_mut(&DriveItemId::new(id)).unwrap().parents = vec![DriveItemId::new(parent)];
    }

    pub fn remove(&mut self, id: &str) {
        self.items.remove(&DriveItemId::new(id));
    }

    pub fn fail_listing(&mut self, folder: &str) {
        self.failing = Some(DriveItemId::new(folder));
    }

    pub fn corrupt_download(&mut self, id: &str) {
        self.corrupt = Some(DriveItemId::new(id));
    }

    fn check(token: &AccessToken) -> Result<(), PortError> {
        if token.secret() == FakeAuth::ACCESS_TOKEN {
            Ok(())
        } else {
            Err(PortError::new("invalid access token"))
        }
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
        if self.failing.as_ref() == Some(folder) {
            return Err(PortError::new("listing failed"));
        }
        Ok(self
            .items
            .values()
            .filter(|item| item.parents.contains(folder))
            .cloned()
            .collect())
    }

    async fn item(&self, token: &AccessToken, id: &DriveItemId) -> Result<DriveItem, PortError> {
        Self::check(token)?;
        self.items
            .get(id)
            .cloned()
            .ok_or_else(|| PortError::new(format!("no item {id}")))
    }

    async fn download<W: FileWriter>(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        out: &mut W,
    ) -> Result<(), PortError> {
        Self::check(token)?;
        if self.corrupt.as_ref() == Some(id) {
            return out.write(b"corrupted").await;
        }
        let content = self
            .contents
            .get(id)
            .ok_or_else(|| PortError::new("no content"))?;
        // Several chunks, like a real download.
        for chunk in content.chunks(3) {
            out.write(chunk).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Dir,
    File {
        content: Vec<u8>,
        modified: SystemTime,
    },
}

/// In-memory filesystem. Paths must be absolute.
#[derive(Default)]
pub struct FakeFileSystem {
    nodes: Mutex<BTreeMap<PathBuf, Node>>,
    clock: AtomicU64,
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

impl FakeFileSystem {
    /// Simulates the user writing a file, with a new modification time.
    pub fn write_local(&self, path: &str, content: &str) {
        let modified = SystemTime::UNIX_EPOCH
            + Duration::from_secs(1_000_000 + self.clock.fetch_add(1, Ordering::Relaxed));
        let path = PathBuf::from(path);
        self.create_dir_all(path.parent().unwrap()).unwrap();
        self.nodes.lock().unwrap().insert(
            path,
            Node::File {
                content: content.as_bytes().to_vec(),
                modified,
            },
        );
    }

    pub fn read(&self, path: &str) -> Option<String> {
        match self.nodes.lock().unwrap().get(Path::new(path)) {
            Some(Node::File { content, .. }) => Some(String::from_utf8(content.clone()).unwrap()),
            _ => None,
        }
    }

    pub fn is_dir(&self, path: &str) -> bool {
        self.nodes.lock().unwrap().get(Path::new(path)) == Some(&Node::Dir)
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
        Node::Dir => LocalMetadata {
            is_dir: true,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
        },
        Node::File { content, modified } => LocalMetadata {
            is_dir: false,
            size: content.len() as u64,
            modified: *modified,
        },
    }
}

impl LocalFileSystem for FakeFileSystem {
    type Writer = FakeWriter;

    fn metadata(&self, path: &Path) -> Result<Option<LocalMetadata>, PortError> {
        Ok(self.nodes.lock().unwrap().get(path).map(meta))
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), PortError> {
        let mut nodes = self.nodes.lock().unwrap();
        for dir in path.ancestors() {
            match nodes.get(dir) {
                Some(Node::File { .. }) => {
                    return Err(PortError::new(format!("{} is a file", dir.display())));
                }
                Some(Node::Dir) => {}
                None => {
                    nodes.insert(dir.to_owned(), Node::Dir);
                }
            }
        }
        Ok(())
    }

    async fn begin_write(&self, path: &Path) -> Result<FakeWriter, PortError> {
        match self.nodes.lock().unwrap().get(path.parent().unwrap()) {
            Some(Node::Dir) => Ok(FakeWriter {
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
pub struct InMemorySyncState(Mutex<HashMap<(AccountId, DriveItemId), SyncedItem>>);

impl SyncStateStore for InMemorySyncState {
    fn items(&self, account: &AccountId) -> Result<Vec<SyncedItem>, PortError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|((a, _), _)| a == account)
            .map(|(_, item)| item.clone())
            .collect())
    }

    fn upsert(&self, account: &AccountId, item: &SyncedItem) -> Result<(), PortError> {
        self.0
            .lock()
            .unwrap()
            .insert((account.clone(), item.id.clone()), item.clone());
        Ok(())
    }

    fn remove(&self, account: &AccountId, item: &DriveItemId) -> Result<(), PortError> {
        self.0
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

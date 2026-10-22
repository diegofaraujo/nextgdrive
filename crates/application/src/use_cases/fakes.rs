use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use nextgdrive_domain::{
    Account, AccountId, Drive, DriveItem, DriveItemId, DriveKind, Email, ItemKind, Settings,
    StorageQuota,
};

use crate::ports::{
    AccessGrant, AccessToken, AccountRepository, Authenticator, AuthorizedTokens, CredentialStore,
    DriveAccountInfo, DriveService, PendingAuthorization, PortError, RefreshToken,
    SettingsRepository,
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
        }
    }

    pub fn with_item(mut self, parent: &str, id: &str, name: &str, kind: ItemKind) -> Self {
        let item = DriveItem {
            id: DriveItemId::new(id),
            name: name.into(),
            kind,
            modified: SystemTime::UNIX_EPOCH,
            parents: vec![DriveItemId::new(parent)],
        };
        self.items.insert(item.id.clone(), item);
        self
    }

    pub fn with_folder(self, parent: &str, id: &str, name: &str) -> Self {
        self.with_item(parent, id, name, ItemKind::Folder)
    }

    pub fn with_file(self, parent: &str, id: &str, name: &str) -> Self {
        self.with_item(parent, id, name, ItemKind::File { size: 1, md5: None })
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
}

pub fn account(id: &str, email: &str) -> Account {
    Account {
        id: AccountId::new(id),
        email: Email::parse(email).unwrap(),
        display_name: None,
    }
}

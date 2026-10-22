use std::collections::HashMap;
use std::sync::Mutex;

use nextgdrive_domain::{Account, AccountId, Email, StorageQuota};

use crate::ports::{
    AccessToken, AccountRepository, Authenticator, AuthorizedTokens, CredentialStore,
    DriveAccountInfo, DriveService, PendingAuthorization, PortError, RefreshToken,
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

/// Signs in immediately, always with the same tokens.
pub struct FakeAuth;

pub struct FakePending;

impl FakeAuth {
    pub const URL: &str = "https://accounts.example/sign-in";
    pub const REFRESH_TOKEN: &str = "refresh";
    pub const ACCESS_TOKEN: &str = "access";
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

    async fn refresh(&self, token: &RefreshToken) -> Result<AccessToken, PortError> {
        if token.secret() == Self::REFRESH_TOKEN {
            Ok(AccessToken::new(Self::ACCESS_TOKEN))
        } else {
            Err(PortError::new("unknown refresh token"))
        }
    }
}

/// Reports the given account to holders of [`FakeAuth::ACCESS_TOKEN`].
pub struct FakeDrive(Account);

impl FakeDrive {
    pub const QUOTA: StorageQuota = StorageQuota {
        usage: 5,
        limit: Some(15),
    };

    pub fn new(account: Account) -> Self {
        Self(account)
    }
}

impl DriveService for FakeDrive {
    async fn account_info(&self, token: &AccessToken) -> Result<DriveAccountInfo, PortError> {
        if token.secret() != FakeAuth::ACCESS_TOKEN {
            return Err(PortError::new("invalid access token"));
        }
        Ok(DriveAccountInfo {
            account: self.0.clone(),
            quota: Self::QUOTA,
        })
    }
}

pub fn account(id: &str, email: &str) -> Account {
    Account {
        id: AccountId::new(id),
        email: Email::parse(email).unwrap(),
        display_name: None,
    }
}

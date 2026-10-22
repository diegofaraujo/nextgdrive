use nextgdrive_domain::Account;

use crate::AppError;
use crate::ports::{
    AccountRepository, Authenticator, CredentialStore, DriveAccountInfo, DriveService,
};

/// Fetches an account's current details (including storage quota) from Google
/// Drive, and updates the stored account if its email or name changed.
pub struct GetAccountStatus<'a, A, D, C, R> {
    auth: &'a A,
    drive: &'a D,
    credentials: &'a C,
    accounts: &'a R,
}

impl<'a, A, D, C, R> GetAccountStatus<'a, A, D, C, R>
where
    A: Authenticator,
    D: DriveService,
    C: CredentialStore,
    R: AccountRepository,
{
    pub fn new(auth: &'a A, drive: &'a D, credentials: &'a C, accounts: &'a R) -> Self {
        Self {
            auth,
            drive,
            credentials,
            accounts,
        }
    }

    pub async fn execute(&self, account: &Account) -> Result<DriveAccountInfo, AppError> {
        let refresh = self
            .credentials
            .load(&account.id)
            .await
            .map_err(AppError::Credentials)?
            .ok_or_else(|| AppError::MissingCredentials(account.email.clone()))?;
        let access = self.auth.refresh(&refresh).await.map_err(AppError::Auth)?;
        let info = self
            .drive
            .account_info(&access)
            .await
            .map_err(AppError::Drive)?;
        if info.account != *account {
            self.accounts
                .save(&info.account)
                .map_err(AppError::Storage)?;
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, InMemoryAccounts, InMemoryCredentials, account,
    };

    #[tokio::test]
    async fn returns_quota_and_updates_changed_details() {
        let stored = account("1", "ana@x.com");
        let current = Account {
            display_name: Some("Ana".into()),
            ..stored.clone()
        };
        let (auth, drive) = (FakeAuth, FakeDrive::new(current.clone()));
        let credentials = InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN);
        let repo = InMemoryAccounts::with([stored.clone()]);

        let info = GetAccountStatus::new(&auth, &drive, &credentials, &repo)
            .execute(&stored)
            .await
            .unwrap();

        assert_eq!(info.quota, FakeDrive::QUOTA);
        assert_eq!(repo.list().unwrap(), [current]);
    }

    #[tokio::test]
    async fn fails_without_credentials() {
        let (auth, drive) = (FakeAuth, FakeDrive::new(account("1", "ana@x.com")));
        let (credentials, repo) = (InMemoryCredentials::default(), InMemoryAccounts::default());

        let result = GetAccountStatus::new(&auth, &drive, &credentials, &repo)
            .execute(&account("1", "ana@x.com"))
            .await;

        assert!(
            matches!(result, Err(AppError::MissingCredentials(e)) if e.as_str() == "ana@x.com")
        );
    }
}

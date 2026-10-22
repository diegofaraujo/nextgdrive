use nextgdrive_domain::Account;

use crate::ports::{
    AccountRepository, Authenticator, CredentialStore, DriveAccountInfo, DriveService,
};
use crate::{AppError, TokenProvider};

/// Fetches an account's current details (including storage quota) from Google
/// Drive, and updates the stored account if its email or name changed.
pub struct GetAccountStatus<'a, A, C, D, R> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    accounts: &'a R,
}

impl<'a, A, C, D, R> GetAccountStatus<'a, A, C, D, R>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    R: AccountRepository,
{
    pub fn new(tokens: &'a TokenProvider<A, C>, drive: &'a D, accounts: &'a R) -> Self {
        Self {
            tokens,
            drive,
            accounts,
        }
    }

    pub async fn execute(&self, account: &Account) -> Result<DriveAccountInfo, AppError> {
        let access = self.tokens.access_token(account).await?;
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
        let tokens = TokenProvider::new(
            FakeAuth::default(),
            InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
        );
        let (drive, repo) = (
            FakeDrive::new(current.clone()),
            InMemoryAccounts::with([stored.clone()]),
        );

        let info = GetAccountStatus::new(&tokens, &drive, &repo)
            .execute(&stored)
            .await
            .unwrap();

        assert_eq!(info.quota, FakeDrive::QUOTA);
        assert_eq!(repo.list().unwrap(), [current]);
    }
}

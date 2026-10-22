use nextgdrive_domain::{Account, Email};

use crate::ports::{AccountRepository, Authenticator, CredentialStore, SettingsRepository};
use crate::{AppError, TokenProvider};

/// Unlinks an account from Next GDrive: forgets its credentials and folder
/// selection. Local files are left untouched.
pub struct RemoveAccount<'a, A, C, R, S> {
    tokens: &'a TokenProvider<A, C>,
    accounts: &'a R,
    settings: &'a S,
}

impl<'a, A, C, R, S> RemoveAccount<'a, A, C, R, S>
where
    A: Authenticator,
    C: CredentialStore,
    R: AccountRepository,
    S: SettingsRepository,
{
    pub fn new(tokens: &'a TokenProvider<A, C>, accounts: &'a R, settings: &'a S) -> Self {
        Self {
            tokens,
            accounts,
            settings,
        }
    }

    /// Returns the removed account.
    pub async fn execute(&self, email: &Email) -> Result<Account, AppError> {
        let account = self
            .accounts
            .find_by_email(email)
            .map_err(AppError::Storage)?
            .ok_or_else(|| AppError::AccountNotFound(email.clone()))?;
        // Credentials go first: if removing the account then fails, it can be
        // re-added, whereas the opposite order would leave an orphan secret.
        self.tokens
            .credentials()
            .delete(&account.id)
            .await
            .map_err(AppError::Credentials)?;
        self.tokens.forget(&account);
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        if settings.selections.remove(&account.id).is_some() {
            self.settings.save(&settings).map_err(AppError::Storage)?;
        }
        self.accounts
            .remove(&account.id)
            .map_err(AppError::Storage)?;
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{
        FakeAuth, InMemoryAccounts, InMemoryCredentials, InMemorySettings, account,
    };
    use nextgdrive_domain::{AccountId, DriveItemId};

    #[tokio::test]
    async fn removes_account_credentials_and_selection() {
        let repo = InMemoryAccounts::with([account("1", "ana@x.com"), account("2", "zoe@x.com")]);
        let tokens = TokenProvider::new(
            FakeAuth::default(),
            InMemoryCredentials::with_token("1", "refresh-1"),
        );
        let settings = InMemorySettings::default();
        settings.update(|s| s.select(&AccountId::new("1"), DriveItemId::new("folder")));

        let removed = RemoveAccount::new(&tokens, &repo, &settings)
            .execute(&Email::parse("ana@x.com").unwrap())
            .await
            .unwrap();

        assert_eq!(removed.id.as_str(), "1");
        assert_eq!(repo.list().unwrap(), [account("2", "zoe@x.com")]);
        assert!(tokens.credentials().is_empty());
        assert!(settings.load().unwrap().selections.is_empty());
    }

    #[tokio::test]
    async fn fails_when_account_does_not_exist() {
        let tokens = TokenProvider::new(FakeAuth::default(), InMemoryCredentials::default());
        let (repo, settings) = (InMemoryAccounts::default(), InMemorySettings::default());

        let result = RemoveAccount::new(&tokens, &repo, &settings)
            .execute(&Email::parse("ana@x.com").unwrap())
            .await;

        assert!(matches!(result, Err(AppError::AccountNotFound(e)) if e.as_str() == "ana@x.com"));
    }
}

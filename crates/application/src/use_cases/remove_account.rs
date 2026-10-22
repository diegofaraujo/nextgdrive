use nextgdrive_domain::{Account, Email};

use crate::AppError;
use crate::ports::{AccountRepository, CredentialStore};

/// Unlinks an account from Next GDrive and forgets its credentials. Local
/// files are left untouched.
pub struct RemoveAccount<'a, C, R> {
    credentials: &'a C,
    accounts: &'a R,
}

impl<'a, C: CredentialStore, R: AccountRepository> RemoveAccount<'a, C, R> {
    pub fn new(credentials: &'a C, accounts: &'a R) -> Self {
        Self {
            credentials,
            accounts,
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
        self.credentials
            .delete(&account.id)
            .await
            .map_err(AppError::Credentials)?;
        self.accounts
            .remove(&account.id)
            .map_err(AppError::Storage)?;
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{InMemoryAccounts, InMemoryCredentials, account};

    #[tokio::test]
    async fn removes_account_and_its_credentials() {
        let repo = InMemoryAccounts::with([account("1", "ana@x.com"), account("2", "zoe@x.com")]);
        let credentials = InMemoryCredentials::with_token("1", "refresh-1");

        let removed = RemoveAccount::new(&credentials, &repo)
            .execute(&Email::parse("ana@x.com").unwrap())
            .await
            .unwrap();

        assert_eq!(removed.id.as_str(), "1");
        assert_eq!(repo.list().unwrap(), [account("2", "zoe@x.com")]);
        assert!(credentials.is_empty());
    }

    #[tokio::test]
    async fn fails_when_account_does_not_exist() {
        let repo = InMemoryAccounts::default();
        let credentials = InMemoryCredentials::default();

        let result = RemoveAccount::new(&credentials, &repo)
            .execute(&Email::parse("ana@x.com").unwrap())
            .await;

        assert!(matches!(result, Err(AppError::AccountNotFound(e)) if e.as_str() == "ana@x.com"));
    }
}

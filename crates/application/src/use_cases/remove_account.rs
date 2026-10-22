use nextgdrive_domain::{Account, Email};

use crate::AppError;
use crate::ports::AccountRepository;

/// Unlinks an account from Next GDrive. Local files are left untouched.
pub struct RemoveAccount<'a, R> {
    accounts: &'a R,
}

impl<'a, R: AccountRepository> RemoveAccount<'a, R> {
    pub fn new(accounts: &'a R) -> Self {
        Self { accounts }
    }

    /// Returns the removed account.
    pub fn execute(&self, email: &Email) -> Result<Account, AppError> {
        let account = self
            .accounts
            .find_by_email(email)?
            .ok_or_else(|| AppError::AccountNotFound(email.clone()))?;
        self.accounts.remove(&account.id)?;
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{InMemoryAccounts, account};

    #[test]
    fn removes_existing_account() {
        let repo = InMemoryAccounts::with([account("1", "ana@x.com"), account("2", "zoe@x.com")]);

        let removed = RemoveAccount::new(&repo)
            .execute(&Email::parse("ana@x.com").unwrap())
            .unwrap();

        assert_eq!(removed.id.as_str(), "1");
        assert_eq!(repo.list().unwrap(), [account("2", "zoe@x.com")]);
    }

    #[test]
    fn fails_when_account_does_not_exist() {
        let repo = InMemoryAccounts::default();

        let result = RemoveAccount::new(&repo).execute(&Email::parse("ana@x.com").unwrap());

        assert!(matches!(result, Err(AppError::AccountNotFound(e)) if e.as_str() == "ana@x.com"));
    }
}

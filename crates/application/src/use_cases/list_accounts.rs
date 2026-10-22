use nextgdrive_domain::Account;

use crate::AppError;
use crate::ports::AccountRepository;

pub struct ListAccounts<'a, R> {
    accounts: &'a R,
}

impl<'a, R: AccountRepository> ListAccounts<'a, R> {
    pub fn new(accounts: &'a R) -> Self {
        Self { accounts }
    }

    /// Returns the linked accounts sorted by email.
    pub fn execute(&self) -> Result<Vec<Account>, AppError> {
        let mut accounts = self.accounts.list().map_err(AppError::Storage)?;
        accounts.sort_by(|a, b| a.email.as_str().cmp(b.email.as_str()));
        Ok(accounts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{InMemoryAccounts, account};

    #[test]
    fn lists_accounts_sorted_by_email() {
        let repo = InMemoryAccounts::with([account("2", "zoe@x.com"), account("1", "ana@x.com")]);

        let emails: Vec<_> = ListAccounts::new(&repo)
            .execute()
            .unwrap()
            .into_iter()
            .map(|a| a.email.to_string())
            .collect();

        assert_eq!(emails, ["ana@x.com", "zoe@x.com"]);
    }
}

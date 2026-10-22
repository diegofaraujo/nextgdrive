use std::cell::RefCell;

use nextgdrive_domain::{Account, AccountId, Email};

use crate::ports::{AccountRepository, RepositoryError};

#[derive(Default)]
pub struct InMemoryAccounts(RefCell<Vec<Account>>);

impl InMemoryAccounts {
    pub fn with(accounts: impl IntoIterator<Item = Account>) -> Self {
        Self(RefCell::new(accounts.into_iter().collect()))
    }
}

impl AccountRepository for InMemoryAccounts {
    fn list(&self) -> Result<Vec<Account>, RepositoryError> {
        Ok(self.0.borrow().clone())
    }

    fn find_by_email(&self, email: &Email) -> Result<Option<Account>, RepositoryError> {
        Ok(self.0.borrow().iter().find(|a| &a.email == email).cloned())
    }

    fn save(&self, account: &Account) -> Result<(), RepositoryError> {
        let mut accounts = self.0.borrow_mut();
        accounts.retain(|a| a.id != account.id);
        accounts.push(account.clone());
        Ok(())
    }

    fn remove(&self, id: &AccountId) -> Result<(), RepositoryError> {
        self.0.borrow_mut().retain(|a| &a.id != id);
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

use std::error::Error;

use nextgdrive_domain::{Account, AccountId, Email};
use thiserror::Error;

/// Failure of the underlying storage. Opaque to the application layer.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct RepositoryError(Box<dyn Error + Send + Sync>);

impl RepositoryError {
    pub fn new(source: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(source.into())
    }
}

/// Persists the accounts linked to Next GDrive.
pub trait AccountRepository {
    fn list(&self) -> Result<Vec<Account>, RepositoryError>;

    fn find_by_email(&self, email: &Email) -> Result<Option<Account>, RepositoryError>;

    /// Inserts the account, or replaces the stored account with the same id.
    fn save(&self, account: &Account) -> Result<(), RepositoryError>;

    /// Removes the account with the given id. Removing a missing account is not an error.
    fn remove(&self, id: &AccountId) -> Result<(), RepositoryError>;
}

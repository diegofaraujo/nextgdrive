use nextgdrive_domain::{Account, AccountId, Email};

use super::PortError;

/// Persists the accounts linked to Next GDrive.
pub trait AccountRepository: Sync {
    fn list(&self) -> Result<Vec<Account>, PortError>;

    fn find_by_email(&self, email: &Email) -> Result<Option<Account>, PortError>;

    /// Inserts the account, or replaces the stored account with the same id.
    fn save(&self, account: &Account) -> Result<(), PortError>;

    /// Removes the account with the given id. Removing a missing account is not an error.
    fn remove(&self, id: &AccountId) -> Result<(), PortError>;
}

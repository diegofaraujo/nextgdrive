use std::future::Future;

use nextgdrive_domain::AccountId;

use super::{PortError, RefreshToken};

/// Keeps each account's refresh token in secure storage.
pub trait CredentialStore: Sync {
    /// Stores the token, replacing any previous token for the account.
    fn store(
        &self,
        account: &AccountId,
        token: &RefreshToken,
    ) -> impl Future<Output = Result<(), PortError>> + Send;

    fn load(
        &self,
        account: &AccountId,
    ) -> impl Future<Output = Result<Option<RefreshToken>, PortError>> + Send;

    /// Deleting a missing token is not an error.
    fn delete(&self, account: &AccountId) -> impl Future<Output = Result<(), PortError>> + Send;
}

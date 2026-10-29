use std::sync::Arc;
use std::time::SystemTime;

use nextgdrive_domain::{AccountId, Drive, DriveItem, DriveItemId};

use super::{ItemChange, PortError};

/// Where an account's index stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStatus {
    /// Change feed position the index is up to date with.
    pub cursor: String,
    /// When the whole Drive was last listed into it.
    pub built: SystemTime,
}

/// A local copy of the metadata of every item in an account's Drive (no
/// content), so its folders can be listed without asking Drive. Kept up to
/// date from Drive's change feed and by what this computer changes there.
pub trait DriveIndex: Sync {
    /// `None` until the whole Drive was listed into it once.
    fn status(&self, account: &AccountId) -> Result<Option<IndexStatus>, PortError>;

    /// Replaces everything known about the account.
    fn rebuild(
        &self,
        account: &AccountId,
        drives: &[Drive],
        items: &[DriveItem],
        cursor: &str,
        built: SystemTime,
    ) -> Result<(), PortError>;

    /// Records changed and removed items, and the feed position reached
    /// (`None` for changes made here, ahead of the feed). Returns the folders
    /// whose content changed: old and new parents of what changed.
    fn apply(
        &self,
        account: &AccountId,
        changes: &[ItemChange],
        cursor: Option<&str>,
    ) -> Result<Vec<DriveItemId>, PortError>;

    fn set_drives(&self, account: &AccountId, drives: &[Drive]) -> Result<(), PortError>;

    /// My Drive first, then shared drives, as `DriveService::drives`.
    fn drives(&self, account: &AccountId) -> Result<Vec<Drive>, PortError>;

    fn children(
        &self,
        account: &AccountId,
        folder: &DriveItemId,
    ) -> Result<Vec<DriveItem>, PortError>;

    /// Deletes the account's index (when the account is removed).
    fn forget(&self, account: &AccountId) -> Result<(), PortError>;
}

impl<T: DriveIndex + Send> DriveIndex for Arc<T> {
    fn status(&self, account: &AccountId) -> Result<Option<IndexStatus>, PortError> {
        (**self).status(account)
    }

    fn rebuild(
        &self,
        account: &AccountId,
        drives: &[Drive],
        items: &[DriveItem],
        cursor: &str,
        built: SystemTime,
    ) -> Result<(), PortError> {
        (**self).rebuild(account, drives, items, cursor, built)
    }

    fn apply(
        &self,
        account: &AccountId,
        changes: &[ItemChange],
        cursor: Option<&str>,
    ) -> Result<Vec<DriveItemId>, PortError> {
        (**self).apply(account, changes, cursor)
    }

    fn set_drives(&self, account: &AccountId, drives: &[Drive]) -> Result<(), PortError> {
        (**self).set_drives(account, drives)
    }

    fn drives(&self, account: &AccountId) -> Result<Vec<Drive>, PortError> {
        (**self).drives(account)
    }

    fn children(
        &self,
        account: &AccountId,
        folder: &DriveItemId,
    ) -> Result<Vec<DriveItem>, PortError> {
        (**self).children(account, folder)
    }

    fn forget(&self, account: &AccountId) -> Result<(), PortError> {
        (**self).forget(account)
    }
}

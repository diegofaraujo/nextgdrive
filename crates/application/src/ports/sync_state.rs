use std::path::PathBuf;
use std::time::SystemTime;

use nextgdrive_domain::{AccountId, DriveItemId};

use super::PortError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncedKind {
    Folder,
    File,
    GoogleDoc,
}

/// What was true of an item when it was last synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedItem {
    pub id: DriveItemId,
    /// Selected folder this item was synced under. Tells "removed from
    /// Drive" apart from "folder deselected" when the item disappears.
    pub selection: DriveItemId,
    /// Relative to the account folder.
    pub path: PathBuf,
    pub kind: SyncedKind,
    pub remote_modified: SystemTime,
    pub remote_md5: Option<String>,
    /// Size and modification time of the local copy right after syncing. If
    /// either differs now, the user changed the file locally.
    pub local_size: u64,
    pub local_modified: SystemTime,
}

/// Remembers the state of synced items, per account.
pub trait SyncStateStore: Sync {
    /// Held while an account is being synced; released on drop.
    type Lock: Send;

    /// Exclusive right to change an account's synced files and state, across
    /// processes. `None` if someone else holds it.
    fn try_lock(&self, account: &AccountId) -> Result<Option<Self::Lock>, PortError>;

    /// Position in Drive's change feed up to which the account is synced.
    fn cursor(&self, account: &AccountId) -> Result<Option<String>, PortError>;

    fn set_cursor(&self, account: &AccountId, cursor: &str) -> Result<(), PortError>;

    fn items(&self, account: &AccountId) -> Result<Vec<SyncedItem>, PortError>;

    fn upsert(&self, account: &AccountId, item: &SyncedItem) -> Result<(), PortError>;

    fn remove(&self, account: &AccountId, item: &DriveItemId) -> Result<(), PortError>;
}

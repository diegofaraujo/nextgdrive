use std::future::Future;

use nextgdrive_domain::{Account, Drive, DriveItem, DriveItemId, StorageQuota, TrashedItem};

use std::time::SystemTime;

use super::{AccessToken, FileReader, FileWriter, PortError};

/// Account details as reported by Google Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveAccountInfo {
    pub account: Account,
    pub quota: StorageQuota,
}

/// Result of reading Drive's change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// Whether anything changed since the given cursor (always true without one).
    pub any: bool,
    /// Where to continue reading next time.
    pub cursor: String,
}

/// One entry of Drive's change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemChange {
    /// Created or changed: how it is now.
    Changed(DriveItem),
    /// Trashed, deleted, or no longer accessible.
    Removed(DriveItemId),
}

/// What changed since a change feed position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedItems {
    pub changes: Vec<ItemChange>,
    /// A shared drive was added, renamed or removed.
    pub drives_changed: bool,
    /// Where to continue reading next time.
    pub cursor: String,
}

/// Where uploaded content goes.
#[derive(Debug, Clone, Copy)]
pub enum UploadTarget<'a> {
    New {
        parent: &'a DriveItemId,
        name: &'a str,
    },
    /// New content for an existing file (kept as a new version in Drive).
    Existing(&'a DriveItemId),
}

/// Google Drive API.
pub trait DriveService: Sync {
    fn account_info(
        &self,
        token: &AccessToken,
    ) -> impl Future<Output = Result<DriveAccountInfo, PortError>> + Send;

    /// My Drive first, then the shared drives the account can access.
    fn drives(
        &self,
        token: &AccessToken,
    ) -> impl Future<Output = Result<Vec<Drive>, PortError>> + Send;

    /// The folder's children if known without asking Drive (e.g. from an
    /// index); then no access token is needed, so they show offline too.
    fn known_children(&self, _folder: &DriveItemId) -> Option<Vec<DriveItem>> {
        None
    }

    /// The drives, if known without asking Drive, as `known_children`.
    fn known_drives(&self) -> Option<Vec<Drive>> {
        None
    }

    /// Items directly inside a folder, excluding trashed ones.
    fn children(
        &self,
        token: &AccessToken,
        folder: &DriveItemId,
    ) -> impl Future<Output = Result<Vec<DriveItem>, PortError>> + Send;

    fn item(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
    ) -> impl Future<Output = Result<DriveItem, PortError>> + Send;

    /// Reads the change feed from `cursor`, or just gets the current position
    /// when there is no cursor yet.
    fn changes_since(
        &self,
        token: &AccessToken,
        cursor: Option<&str>,
    ) -> impl Future<Output = Result<Changes, PortError>> + Send;

    /// What changed since `cursor` (from `changes_since`), item by item.
    fn changed_items(
        &self,
        token: &AccessToken,
        cursor: &str,
    ) -> impl Future<Output = Result<ChangedItems, PortError>> + Send;

    /// Every item in a drive, at any depth, excluding trashed ones. For My
    /// Drive it may include items shared with the user that are elsewhere.
    fn items_in(
        &self,
        token: &AccessToken,
        drive: &Drive,
    ) -> impl Future<Output = Result<Vec<DriveItem>, PortError>> + Send;

    /// Streams a file's content into `out`.
    fn download<W: FileWriter>(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        out: &mut W,
    ) -> impl Future<Output = Result<(), PortError>> + Send;

    fn create_folder(
        &self,
        token: &AccessToken,
        parent: &DriveItemId,
        name: &str,
    ) -> impl Future<Output = Result<DriveItem, PortError>> + Send;

    /// Uploads `size` bytes from `content`; Drive records `modified` as the
    /// file's modification time.
    fn upload<R: FileReader>(
        &self,
        token: &AccessToken,
        target: UploadTarget<'_>,
        size: u64,
        modified: SystemTime,
        content: &mut R,
    ) -> impl Future<Output = Result<DriveItem, PortError>> + Send;

    /// Moves an item from one folder to another and/or renames it.
    fn move_item(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        from: &DriveItemId,
        to: &DriveItemId,
        name: &str,
    ) -> impl Future<Output = Result<DriveItem, PortError>> + Send;

    /// Moves an item to the trash, from which it can be restored for 30 days.
    /// Next GDrive never deletes anything permanently.
    fn trash(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
    ) -> impl Future<Output = Result<(), PortError>> + Send;

    /// Takes an item someone else owns out of the user's My Drive.
    fn remove_from_my_drive(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        parent: &DriveItemId,
    ) -> impl Future<Output = Result<(), PortError>> + Send;

    /// Items the user trashed themselves; what was inside a trashed folder
    /// is not listed separately (restoring the folder brings it back).
    fn trashed_items(
        &self,
        token: &AccessToken,
    ) -> impl Future<Output = Result<Vec<TrashedItem>, PortError>> + Send;

    /// Takes an item (and everything inside it) out of the trash, back where it was.
    fn restore(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
    ) -> impl Future<Output = Result<DriveItem, PortError>> + Send;

    /// Up to `len` bytes of a file's content from `offset`; fewer at the end.
    fn download_range(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        offset: u64,
        len: u64,
    ) -> impl Future<Output = Result<Vec<u8>, PortError>> + Send;
}

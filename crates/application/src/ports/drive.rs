use std::future::Future;

use nextgdrive_domain::{Account, Drive, DriveItem, DriveItemId, StorageQuota};

use super::{AccessToken, FileWriter, PortError};

/// Account details as reported by Google Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveAccountInfo {
    pub account: Account,
    pub quota: StorageQuota,
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

    /// Streams a file's content into `out`.
    fn download<W: FileWriter>(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        out: &mut W,
    ) -> impl Future<Output = Result<(), PortError>> + Send;
}

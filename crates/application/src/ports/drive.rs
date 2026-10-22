use std::future::Future;

use nextgdrive_domain::{Account, StorageQuota};

use super::{AccessToken, PortError};

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
}

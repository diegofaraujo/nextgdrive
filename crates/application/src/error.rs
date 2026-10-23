use std::path::PathBuf;

use nextgdrive_domain::{DriveItemId, Email};
use thiserror::Error;

use crate::ports::PortError;

/// Each variant tells front-ends what failed, so they can show a suitable message.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("no account with email {0}")]
    AccountNotFound(Email),
    #[error("no stored credentials for {0}; add the account again")]
    MissingCredentials(Email),
    #[error("{0} is already being synced")]
    SyncInProgress(Email),
    /// A synced folder vanished as a whole (e.g. an unmounted disk). Syncing
    /// would send everything in it to the trash, so nothing is done.
    #[error("synced folder not found: {0}")]
    LocalFolderMissing(PathBuf),
    /// Many files were deleted locally at once; waiting for the user to confirm.
    #[error("{count} files were deleted on this computer; confirm to move them to the Drive trash")]
    MassDeletion { count: usize },
    #[error("sync folder must be an absolute path: {0}")]
    InvalidSyncRoot(PathBuf),
    #[error("not a folder: {0}")]
    NotAFolder(DriveItemId),
    #[error("a folder already exists at {0}")]
    FolderExists(PathBuf),
    #[error("storage: {0}")]
    Storage(#[source] PortError),
    #[error("sign-in: {0}")]
    Auth(#[source] PortError),
    #[error("credentials: {0}")]
    Credentials(#[source] PortError),
    #[error("Google Drive: {0}")]
    Drive(#[source] PortError),
}

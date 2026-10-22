use nextgdrive_domain::Email;
use thiserror::Error;

use crate::ports::PortError;

/// Each variant tells front-ends what failed, so they can show a suitable message.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("no account with email {0}")]
    AccountNotFound(Email),
    #[error("no stored credentials for {0}; add the account again")]
    MissingCredentials(Email),
    #[error("storage: {0}")]
    Storage(#[source] PortError),
    #[error("sign-in: {0}")]
    Auth(#[source] PortError),
    #[error("credentials: {0}")]
    Credentials(#[source] PortError),
    #[error("Google Drive: {0}")]
    Drive(#[source] PortError),
}

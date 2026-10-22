use nextgdrive_domain::Email;
use thiserror::Error;

use crate::ports::RepositoryError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("no account with email {0}")]
    AccountNotFound(Email),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

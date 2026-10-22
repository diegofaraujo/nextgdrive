use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("invalid email address: {0:?}")]
    InvalidEmail(String),
}

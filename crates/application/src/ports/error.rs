use std::error::Error;

use thiserror::Error;

/// Failure of an adapter behind a port. Opaque to the application layer.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct PortError(Box<dyn Error + Send + Sync>);

impl PortError {
    pub fn new(source: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(source.into())
    }
}

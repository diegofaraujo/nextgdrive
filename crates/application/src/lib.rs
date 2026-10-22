//! Use cases of Next GDrive and the ports (traits) they need from the
//! outside world. Adapters implement the ports; GUI and CLI call the use cases.

mod error;
pub mod ports;
mod tokens;
pub mod use_cases;

pub use error::AppError;
pub use tokens::TokenProvider;

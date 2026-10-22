//! Core entities and business rules of Next GDrive.
//!
//! This crate is pure: it performs no I/O and depends on no infrastructure
//! (Google APIs, filesystem, GUI, CLI).

mod account;
mod error;

pub use account::{Account, AccountId, Email};
pub use error::DomainError;

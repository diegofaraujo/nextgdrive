//! Core entities and business rules of Next GDrive.
//!
//! This crate is pure: it performs no I/O and depends on no infrastructure
//! (Google APIs, filesystem, GUI, CLI).

mod account;
mod drive;
mod error;
pub mod local_name;
mod quota;
mod settings;

pub use account::{Account, AccountId, Email};
pub use drive::{Capabilities, Drive, DriveItem, DriveItemId, DriveKind, ItemKind, TrashedItem};
pub use error::DomainError;
pub use quota::StorageQuota;
pub use settings::Settings;

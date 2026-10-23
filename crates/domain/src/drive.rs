use std::fmt;
use std::time::SystemTime;

/// Identifier of a file or folder in Google Drive. Stable across renames and moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DriveItemId(String);

impl DriveItemId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DriveItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A top-level drive: the account's My Drive, or a shared drive it can access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// Id of the drive's root folder. For shared drives it is also the drive id.
    pub root: DriveItemId,
    pub kind: DriveKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveKind {
    MyDrive,
    Shared { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveItem {
    pub id: DriveItemId,
    pub name: String,
    pub kind: ItemKind,
    pub modified: SystemTime,
    pub parents: Vec<DriveItemId>,
    pub capabilities: Capabilities,
}

/// What the user may do with an item, as Drive reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Change its content or name.
    pub edit: bool,
    /// Move it to the trash: owners in My Drive, managers in shared drives.
    pub trash: bool,
    /// Take it out of the user's My Drive without deleting it: what
    /// "deleting" means for items other people own.
    pub remove_from_my_drive: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            edit: true,
            trash: true,
            remove_from_my_drive: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    Folder,
    /// Regular file with downloadable content.
    File {
        size: u64,
        md5: Option<String>,
    },
    /// Google Docs, Sheets, Slides, etc.: no file content, only a web page.
    GoogleDoc {
        url: String,
    },
    /// Drive shortcut to another item.
    Shortcut,
    /// Content the user cannot download (e.g. download disabled by the owner).
    NotDownloadable,
}

/// An item the user moved to the trash. Drive deletes it for good after 30 days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedItem {
    pub item: DriveItem,
    /// Drive only reports it for items in shared drives.
    pub trashed_at: Option<SystemTime>,
}

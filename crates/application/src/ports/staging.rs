use std::time::SystemTime;

use nextgdrive_domain::DriveItemId;

use super::PortError;

/// Where a staged file's content goes once uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedTarget {
    /// A new version of this Drive file, which was at `base_version` (its MD5)
    /// when editing began.
    Existing {
        id: DriveItemId,
        base_version: String,
    },
    /// A new file in this Drive folder.
    New { parent: DriveItemId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    /// Unique, and stable across restarts.
    pub key: String,
    /// Name in Drive.
    pub name: String,
    pub target: StagedTarget,
    pub modified: SystemTime,
}

/// Local content of streamed files being written, kept until it is safely
/// in Drive. Survives restarts, so nothing written is lost if uploading fails.
pub trait StagingArea: Sync {
    /// Starts an empty staged file.
    fn create(&self, file: &StagedFile) -> Result<(), PortError>;

    /// Saves changed details (name, target) of a staged file.
    fn update(&self, file: &StagedFile) -> Result<(), PortError>;

    /// Everything staged, e.g. left from before a restart.
    fn list(&self) -> Result<Vec<StagedFile>, PortError>;

    fn len(&self, key: &str) -> Result<u64, PortError>;

    fn read_at(&self, key: &str, offset: u64, size: usize) -> Result<Vec<u8>, PortError>;

    fn write_at(&self, key: &str, offset: u64, data: &[u8]) -> Result<(), PortError>;

    fn set_len(&self, key: &str, len: u64) -> Result<(), PortError>;

    fn remove(&self, key: &str) -> Result<(), PortError>;
}

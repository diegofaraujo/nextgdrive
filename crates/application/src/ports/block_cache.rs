use nextgdrive_domain::DriveItemId;

use super::PortError;

/// One block of a file's content, as it was at one version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockKey {
    pub file: DriveItemId,
    /// Changes whenever the content does (Drive's MD5), so a stale block is
    /// never served for a newer version.
    pub version: String,
    pub index: u64,
}

/// Keeps downloaded blocks of streamed files. Implementations bound their
/// size and drop what was used least recently.
pub trait BlockCache: Sync {
    fn get(&self, key: &BlockKey) -> Result<Option<Vec<u8>>, PortError>;

    fn put(&self, key: &BlockKey, data: &[u8]) -> Result<(), PortError>;

    /// Whether the block is cached, without reading it.
    fn contains(&self, key: &BlockKey) -> bool;

    /// Files (at these versions) made available offline: their blocks are
    /// never evicted, and do not count toward the cache limit. Replaces the
    /// previous set.
    fn pin(&self, files: &[(DriveItemId, String)]);

    /// Bytes the cache holds now, offline items included.
    fn usage(&self) -> u64;

    /// Removes every block except those of `keep`. Returns the bytes freed.
    fn clear_except(&self, keep: &[(DriveItemId, String)]) -> Result<u64, PortError>;
}

use std::future::Future;
use std::path::Path;
use std::time::SystemTime;

use super::PortError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalMetadata {
    pub is_dir: bool,
    pub size: u64,
    pub modified: SystemTime,
    /// Survives renames and moves within a filesystem, which is how moved
    /// files are recognized.
    pub inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub metadata: LocalMetadata,
}

/// Content of a local file being uploaded.
pub trait FileReader: Send {
    /// Fills `buf` as far as possible; returns 0 at the end of the file.
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, PortError>> + Send;
}

/// A file being written. Its content only appears at the target path once
/// [`LocalFileSystem::commit`] succeeds; dropping it discards the content.
pub trait FileWriter: Send {
    fn write(&mut self, chunk: &[u8]) -> impl Future<Output = Result<(), PortError>> + Send;

    /// MD5 of everything written so far, in lowercase hex (as Google Drive reports it).
    fn md5_hex(&self) -> String;
}

/// The local side of the sync. Paths are absolute.
pub trait LocalFileSystem: Sync {
    type Writer: FileWriter;
    type Reader: FileReader;

    /// Files and directories directly inside `path`. Symbolic links and names
    /// that are not valid UTF-8 are left out: they cannot be synced.
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, PortError>;

    fn open_read(
        &self,
        path: &Path,
    ) -> impl Future<Output = Result<Self::Reader, PortError>> + Send;

    /// `None` if nothing exists at the path.
    fn metadata(&self, path: &Path) -> Result<Option<LocalMetadata>, PortError>;

    fn create_dir_all(&self, path: &Path) -> Result<(), PortError>;

    fn begin_write(
        &self,
        path: &Path,
    ) -> impl Future<Output = Result<Self::Writer, PortError>> + Send;

    /// Moves the written content into place (replacing any file there) and
    /// sets its modification time.
    fn commit(
        &self,
        writer: Self::Writer,
        modified: SystemTime,
    ) -> impl Future<Output = Result<LocalMetadata, PortError>> + Send;

    /// MD5 of an existing file, in lowercase hex.
    fn md5_hex(&self, path: &Path) -> impl Future<Output = Result<String, PortError>> + Send;

    /// Whether an existing file holds exactly `expected`.
    fn has_content(
        &self,
        path: &Path,
        expected: &[u8],
    ) -> impl Future<Output = Result<bool, PortError>> + Send;

    /// Fails if something already exists at `to`.
    fn rename(&self, from: &Path, to: &Path) -> Result<(), PortError>;

    fn remove_file(&self, path: &Path) -> Result<(), PortError>;

    /// Removes the directory only if it is empty. Returns whether it was removed.
    fn remove_dir_if_empty(&self, path: &Path) -> Result<bool, PortError>;
}

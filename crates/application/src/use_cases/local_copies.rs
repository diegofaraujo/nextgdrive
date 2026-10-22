//! Safe removal of local copies, shared by sync and deselection so both
//! follow the same rule: only what is exactly as last synced is removed.

use std::path::Path;

use crate::ports::{LocalFileSystem, LocalMetadata, PortError, SyncedItem, SyncedKind};

pub(crate) enum Removal {
    Removed,
    /// Changed since the last sync; left in place.
    KeptChanged,
    /// A folder that still holds files that were not removed.
    KeptNotEmpty,
    Absent,
}

pub(crate) fn remove_local_copy<F: LocalFileSystem>(
    fs: &F,
    path: &Path,
    synced: &SyncedItem,
) -> Result<Removal, PortError> {
    Ok(match (synced.kind, fs.metadata(path)?) {
        (_, None) => Removal::Absent,
        (SyncedKind::Folder, Some(_)) => {
            if fs.remove_dir_if_empty(path)? {
                Removal::Removed
            } else {
                Removal::KeptNotEmpty
            }
        }
        (_, Some(meta)) if unchanged(synced, &meta) => {
            fs.remove_file(path)?;
            Removal::Removed
        }
        (_, Some(_)) => Removal::KeptChanged,
    })
}

/// Whether the local copy is still exactly what was written at the last sync.
pub(crate) fn unchanged(synced: &SyncedItem, local: &LocalMetadata) -> bool {
    !local.is_dir && local.size == synced.local_size && local.modified == synced.local_modified
}

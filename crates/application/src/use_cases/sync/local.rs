//! Scanning what exists locally inside the selected folders.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nextgdrive_domain::local_name::is_ignored;

use super::remote::Root;
use crate::AppError;
use crate::ports::{LocalFileSystem, LocalMetadata};

/// Everything under each root (roots included), keyed by path relative to the
/// account folder. Ignored names, and whatever is inside ignored folders, are
/// left out. A missing root just contributes nothing.
pub(super) fn scan<F: LocalFileSystem>(
    fs: &F,
    account_dir: &Path,
    roots: &[Root],
) -> Result<BTreeMap<PathBuf, LocalMetadata>, AppError> {
    let mut found = BTreeMap::new();
    for root in roots {
        let Some(meta) = fs
            .metadata(&account_dir.join(&root.path))
            .map_err(AppError::Storage)?
        else {
            continue;
        };
        found.insert(root.path.clone(), meta);
        if !meta.is_dir {
            continue;
        }
        let mut pending = vec![root.path.clone()];
        while let Some(dir) = pending.pop() {
            for entry in fs
                .list_dir(&account_dir.join(&dir))
                .map_err(AppError::Storage)?
            {
                if is_ignored(&entry.name) {
                    continue;
                }
                let path = dir.join(&entry.name);
                if entry.metadata.is_dir {
                    pending.push(path.clone());
                }
                found.insert(path, entry.metadata);
            }
        }
    }
    Ok(found)
}

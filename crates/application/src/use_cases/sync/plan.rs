//! Deciding what to do: a pure function of what Drive has, what exists
//! locally, and what was true at the last sync (the "base").
//!
//! For each item, both sides are compared against the base:
//!
//! | local \ Drive     | unchanged      | changed / moved     | gone          |
//! |-------------------|----------------|---------------------|---------------|
//! | unchanged         | —              | download / move     | remove local  |
//! | changed           | upload         | conflict copy       | upload as new |
//! | deleted           | trash in Drive | download (Drive wins) | forget      |
//! | moved (by inode)  | move in Drive  | follow Drive        | upload as new |
//!
//! Untracked local items become uploads; untracked Drive items, downloads.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use nextgdrive_domain::{DriveItem, DriveItemId, ItemKind};

use super::ConflictReason;
use super::remote::{Root, Wanted};
use crate::ports::{LocalMetadata, SyncedItem, SyncedKind};
use crate::use_cases::local_copies::unchanged;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Op {
    /// Stop tracking (deselected, or gone on both sides).
    Forget(DriveItemId),
    /// Gone from Drive: remove the local copy if it is unchanged.
    RemoveLocal {
        id: DriveItemId,
        path: PathBuf,
    },
    /// Deleted locally: move to the Drive trash (or out of My Drive).
    Trash(DriveItemId),
    /// Moved in Drive: move the local copy.
    MoveLocal {
        id: DriveItemId,
        from: PathBuf,
        to: PathBuf,
    },
    /// Moved locally: move it in Drive to `to`.
    MoveRemote {
        id: DriveItemId,
        to: PathBuf,
    },
    MakeLocalDir(DriveItemId),
    MakeRemoteDir {
        path: PathBuf,
        selection: DriveItemId,
    },
    Download(DriveItemId),
    /// An untracked local file is where a Drive file goes: take it over if
    /// identical, otherwise keep it as a conflict copy.
    DownloadOrAdopt(DriveItemId),
    UploadUpdate {
        id: DriveItemId,
        path: PathBuf,
    },
    UploadNew {
        path: PathBuf,
        selection: DriveItemId,
    },
    /// Changed on both sides: keep the local version as a copy, then download.
    ConflictCopy {
        id: DriveItemId,
        path: PathBuf,
    },
    /// Nothing changed but the recorded path or selection.
    Record(DriveItemId),
    /// Leave it alone and report it.
    Keep {
        path: PathBuf,
        reason: ConflictReason,
    },
}

pub(super) struct Plan {
    pub ops: Vec<Op>,
    /// Drive's view after the planned moves, by item.
    pub remote: BTreeMap<DriveItemId, Wanted>,
    /// Files (Google Docs included) that would go to the trash.
    pub deletions: usize,
    /// Files tracked before this sync, to judge whether deletions are massive.
    pub tracked_files: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MissingFiles {
    /// Local deletions go to the Drive trash.
    Trash,
    /// Local deletions are undone by downloading again.
    Restore,
}

pub(super) fn plan(
    mut remote: BTreeMap<DriveItemId, Wanted>,
    roots: &[Root],
    mut local: BTreeMap<PathBuf, LocalMetadata>,
    state: &HashMap<DriveItemId, SyncedItem>,
    is_selected: &dyn Fn(&DriveItemId) -> bool,
    missing: MissingFiles,
) -> Plan {
    let mut planner = Planner::default();
    let mut base: BTreeMap<DriveItemId, SyncedItem> = BTreeMap::new();
    for item in state.values() {
        if is_selected(&item.selection) {
            base.insert(item.id.clone(), item.clone());
        } else {
            planner.delete.push(Op::Forget(item.id.clone()));
        }
    }
    let tracked_files = base
        .values()
        .filter(|b| b.kind != SyncedKind::Folder)
        .count();

    follow_folder_moves(&mut planner, &mut base, &mut remote, &mut local);

    // Local paths no tracked or Drive item accounts for: moved or new items.
    let known: HashSet<PathBuf> = base
        .values()
        .map(|b| b.path.clone())
        .chain(remote.values().map(|w| w.path.clone()))
        .collect();
    let by_inode: HashMap<u64, PathBuf> = local
        .iter()
        .filter(|(path, meta)| !meta.is_dir && !known.contains(*path))
        .map(|(path, meta)| (meta.inode, path.clone()))
        .collect();

    let handled = trash_deleted_folders(&mut planner, &base, &remote, &local, missing);
    for b in base.values().filter(|b| !handled.contains(&b.id)) {
        let r = remote.get(&b.id);
        if b.kind == SyncedKind::Folder {
            planner.tracked_folder(b, r, local.get(&b.path));
        } else {
            let moved_to = by_inode
                .get(&b.local_inode)
                .filter(|path| local.get(*path).is_some_and(|m| unchanged(b, m)));
            planner.tracked_file(b, r, local.get(&b.path), moved_to, missing);
        }
    }

    for (id, w) in remote.iter().filter(|(id, _)| !base.contains_key(*id)) {
        planner.new_remote(id, w, local.get(&w.path));
    }

    let mut new_local: Vec<(&PathBuf, &LocalMetadata)> = local
        .iter()
        .filter(|(path, _)| !known.contains(*path) && !planner.claimed.contains(*path))
        .collect();
    new_local.sort_by_key(|(path, _)| path.components().count());
    for (path, meta) in new_local {
        let Some(root) = root_of(roots, path) else {
            continue;
        };
        let selection = root.selection.clone();
        let path = path.clone();
        if meta.is_dir {
            planner.dirs.push(Op::MakeRemoteDir { path, selection });
        } else {
            planner.files.push(Op::UploadNew { path, selection });
        }
    }

    planner.finish(remote, tracked_files)
}

#[derive(Default)]
struct Planner {
    /// Phases, run in this order.
    delete: Vec<Op>,
    dirs: Vec<Op>,
    files: Vec<Op>,
    /// Folders removed after their content is gone, deepest first.
    cleanup: Vec<(PathBuf, Op)>,
    /// Local paths already accounted for.
    claimed: HashSet<PathBuf>,
    /// Local copies removed before anything is downloaded: a new Drive item
    /// with the same name may take their place in the same sync.
    freed: HashSet<PathBuf>,
    deletions: usize,
}

impl Planner {
    fn tracked_folder(
        &mut self,
        b: &SyncedItem,
        r: Option<&Wanted>,
        local: Option<&LocalMetadata>,
    ) {
        match (local, r) {
            (Some(meta), Some(w)) if meta.is_dir => {
                self.claimed.insert(b.path.clone());
                if w.path != b.path {
                    // Drive moved it but the local move was not possible.
                    self.keep(&b.path, ConflictReason::TypeClash);
                } else if w.selection != b.selection || b.local_inode == 0 {
                    self.files.push(Op::Record(b.id.clone()));
                }
            }
            (Some(meta), None) if meta.is_dir => {
                self.claimed.insert(b.path.clone());
                self.cleanup.push((
                    b.path.clone(),
                    Op::RemoveLocal {
                        id: b.id.clone(),
                        path: b.path.clone(),
                    },
                ));
            }
            (Some(_), _) => self.keep(&b.path, ConflictReason::TypeClash),
            (None, None) => self.delete.push(Op::Forget(b.id.clone())),
            // Deleted locally but not trashed (see `trash_deleted_folders`): recreate.
            (None, Some(_)) => self.dirs.push(Op::MakeLocalDir(b.id.clone())),
        }
    }

    fn tracked_file(
        &mut self,
        b: &SyncedItem,
        r: Option<&Wanted>,
        local: Option<&LocalMetadata>,
        moved_to: Option<&PathBuf>,
        missing: MissingFiles,
    ) {
        let doc = b.kind == SyncedKind::GoogleDoc;
        let id = || b.id.clone();
        match local {
            Some(meta) if meta.is_dir => self.keep(&b.path, ConflictReason::TypeClash),
            Some(meta) => {
                self.claimed.insert(b.path.clone());
                let changed_locally = !unchanged(b, meta);
                let Some(w) = r else {
                    if changed_locally && !doc {
                        // Edited here, deleted in Drive: the edit wins.
                        self.files.push(Op::UploadNew {
                            path: b.path.clone(),
                            selection: b.selection.clone(),
                        });
                        self.delete.push(Op::Forget(id()));
                    } else {
                        self.delete.push(Op::RemoveLocal {
                            id: id(),
                            path: b.path.clone(),
                        });
                        self.free(&b.path);
                    }
                    return;
                };
                let moved = w.path != b.path;
                let changed_remotely = remote_changed(b, &w.item);
                if moved {
                    self.claimed.insert(w.path.clone());
                }
                if doc {
                    // Shortcuts are generated: Drive always wins, and a moved
                    // or renamed one is written afresh.
                    if moved && !changed_locally {
                        self.delete.push(Op::RemoveLocal {
                            id: id(),
                            path: b.path.clone(),
                        });
                    }
                    if moved || changed_remotely || changed_locally {
                        self.files.push(Op::Download(id()));
                    }
                } else if changed_locally && changed_remotely {
                    self.files.push(Op::ConflictCopy {
                        id: id(),
                        path: b.path.clone(),
                    });
                } else {
                    if moved {
                        self.files.push(Op::MoveLocal {
                            id: id(),
                            from: b.path.clone(),
                            to: w.path.clone(),
                        });
                    }
                    if changed_remotely {
                        self.files.push(Op::Download(id()));
                    } else if changed_locally && w.item.capabilities.edit {
                        self.files.push(Op::UploadUpdate {
                            id: id(),
                            path: w.path.clone(),
                        });
                    } else if changed_locally {
                        self.keep(&w.path, ConflictReason::ReadOnly);
                    } else if !moved && (w.selection != b.selection || b.local_inode == 0) {
                        // Also fills in inodes unknown to older versions.
                        self.files.push(Op::Record(id()));
                    }
                }
            }
            None => match (moved_to, r) {
                (Some(to), None) => {
                    self.claimed.insert(to.clone());
                    self.files.push(Op::UploadNew {
                        path: to.clone(),
                        selection: b.selection.clone(),
                    });
                    self.delete.push(Op::Forget(id()));
                }
                (Some(to), Some(w)) if w.path == b.path && !remote_changed(b, &w.item) => {
                    self.claimed.insert(to.clone());
                    self.files.push(Op::MoveRemote {
                        id: id(),
                        to: to.clone(),
                    });
                }
                // Moved on both sides, or moved here and changed in Drive: follow Drive.
                (Some(to), Some(w)) => {
                    self.claimed.insert(to.clone());
                    self.claimed.insert(w.path.clone());
                    self.files.push(Op::MoveLocal {
                        id: id(),
                        from: to.clone(),
                        to: w.path.clone(),
                    });
                    if remote_changed(b, &w.item) {
                        self.files.push(Op::Download(id()));
                    }
                }
                (None, None) => self.delete.push(Op::Forget(id())),
                // Changed or moved in Drive since: someone still wants it.
                (None, Some(w))
                    if missing == MissingFiles::Restore
                        || w.path != b.path
                        || remote_changed(b, &w.item) =>
                {
                    self.claimed.insert(w.path.clone());
                    self.files.push(Op::Download(id()));
                }
                (None, Some(_)) => {
                    self.deletions += 1;
                    self.delete.push(Op::Trash(id()));
                }
            },
        }
    }

    /// The local copy at `path` is removed in the delete phase, which runs
    /// before any download.
    fn free(&mut self, path: &Path) {
        self.claimed.remove(path);
        self.freed.insert(path.to_owned());
    }

    fn new_remote(&mut self, id: &DriveItemId, w: &Wanted, local: Option<&LocalMetadata>) {
        let local = if self.freed.contains(&w.path) {
            None
        } else {
            local
        };
        let taken = self.claimed.contains(&w.path);
        self.claimed.insert(w.path.clone());
        let folder = w.item.kind == ItemKind::Folder;
        match local {
            _ if taken => self.keep(&w.path, ConflictReason::TypeClash),
            None if folder => self.dirs.push(Op::MakeLocalDir(id.clone())),
            None => self.files.push(Op::Download(id.clone())),
            Some(meta) if meta.is_dir && folder => self.files.push(Op::Record(id.clone())),
            Some(meta) if !meta.is_dir && !folder => {
                self.files.push(Op::DownloadOrAdopt(id.clone()))
            }
            Some(_) => self.keep(&w.path, ConflictReason::TypeClash),
        }
    }

    fn keep(&mut self, path: &Path, reason: ConflictReason) {
        self.claimed.insert(path.to_owned());
        self.files.push(Op::Keep {
            path: path.to_owned(),
            reason,
        });
    }

    fn finish(mut self, remote: BTreeMap<DriveItemId, Wanted>, tracked_files: usize) -> Plan {
        // Parents before children; moves were already planned in that order.
        self.dirs.sort_by_key(|op| match op {
            Op::MakeRemoteDir { path, .. } => path.components().count(),
            Op::MakeLocalDir(id) => remote.get(id).map_or(0, |w| w.path.components().count()),
            _ => 0,
        });
        self.cleanup
            .sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
        let ops = self
            .delete
            .into_iter()
            .chain(self.dirs)
            .chain(self.files)
            .chain(self.cleanup.into_iter().map(|(_, op)| op))
            .collect();
        Plan {
            ops,
            remote,
            deletions: self.deletions,
            tracked_files,
        }
    }
}

/// Folder moves are resolved first, shallowest first, and every path below a
/// moved folder is rewritten, so the items inside do not look moved one by one.
fn follow_folder_moves(
    planner: &mut Planner,
    base: &mut BTreeMap<DriveItemId, SyncedItem>,
    remote: &mut BTreeMap<DriveItemId, Wanted>,
    local: &mut BTreeMap<PathBuf, LocalMetadata>,
) {
    let mut folders: Vec<DriveItemId> = base
        .values()
        .filter(|b| b.kind == SyncedKind::Folder)
        .map(|b| b.id.clone())
        .collect();
    folders.sort_by_key(|id| base[id].path.components().count());
    for id in folders {
        let (b_path, inode) = (base[&id].path.clone(), base[&id].local_inode);
        let Some(w_path) = remote.get(&id).map(|w| w.path.clone()) else {
            continue;
        };
        let here = local.get(&b_path).is_some_and(|m| m.is_dir);
        if w_path != b_path && here && !local.contains_key(&w_path) {
            // Moved in Drive.
            planner.dirs.push(Op::MoveLocal {
                id: id.clone(),
                from: b_path.clone(),
                to: w_path.clone(),
            });
            rename_prefix(local, &b_path, &w_path);
            for b in base.values_mut() {
                b.path = replace_prefix(&b.path, &b_path, &w_path);
            }
        } else if w_path == b_path && !here {
            // Moved locally?
            let known: HashSet<&PathBuf> = remote.values().map(|w| &w.path).collect();
            let found = local
                .iter()
                .find(|(path, meta)| meta.is_dir && meta.inode == inode && !known.contains(path))
                .map(|(path, _)| path.clone());
            if let Some(to) = found {
                planner.dirs.push(Op::MoveRemote {
                    id: id.clone(),
                    to: to.clone(),
                });
                for w in remote.values_mut() {
                    w.path = replace_prefix(&w.path, &b_path, &to);
                }
                for b in base.values_mut() {
                    b.path = replace_prefix(&b.path, &b_path, &to);
                }
            }
        }
    }
}

/// A folder deleted locally goes to the trash as one item, but only if all
/// of it is gone and nothing in it changed in Drive meanwhile (that change
/// would be lost). Returns the items handled here.
fn trash_deleted_folders(
    planner: &mut Planner,
    base: &BTreeMap<DriveItemId, SyncedItem>,
    remote: &BTreeMap<DriveItemId, Wanted>,
    local: &BTreeMap<PathBuf, LocalMetadata>,
    missing: MissingFiles,
) -> HashSet<DriveItemId> {
    let mut handled = HashSet::new();
    if missing == MissingFiles::Restore {
        return handled;
    }
    let mut folders: Vec<&SyncedItem> = base
        .values()
        .filter(|b| b.kind == SyncedKind::Folder && !local.contains_key(&b.path))
        .filter(|b| remote.get(&b.id).is_some_and(|w| w.path == b.path))
        .collect();
    folders.sort_by_key(|b| b.path.components().count());
    for folder in folders {
        if handled.contains(&folder.id) {
            continue; // Inside a folder already trashed.
        }
        let inside = |path: &Path| path != folder.path && path.starts_with(&folder.path);
        let tracked: Vec<&SyncedItem> = base.values().filter(|b| inside(&b.path)).collect();
        let untouched = tracked.iter().all(|b| {
            !local.contains_key(&b.path)
                && remote
                    .get(&b.id)
                    .is_none_or(|w| w.path == b.path && !remote_changed(b, &w.item))
        });
        let added_in_drive = remote
            .iter()
            .any(|(id, w)| inside(&w.path) && !base.contains_key(id));
        if !untouched || added_in_drive {
            continue;
        }
        planner.deletions += tracked
            .iter()
            .filter(|b| b.kind != SyncedKind::Folder)
            .count();
        planner.delete.push(Op::Trash(folder.id.clone()));
        handled.insert(folder.id.clone());
        for b in tracked {
            handled.insert(b.id.clone());
            planner.delete.push(Op::Forget(b.id.clone()));
        }
    }
    handled
}

pub(super) fn remote_changed(synced: &SyncedItem, item: &DriveItem) -> bool {
    match &item.kind {
        ItemKind::File { md5: Some(md5), .. } => synced.remote_md5.as_ref() != Some(md5),
        _ => synced.remote_modified != item.modified,
    }
}

fn root_of<'r>(roots: &'r [Root], path: &Path) -> Option<&'r Root> {
    roots
        .iter()
        .filter(|root| path.starts_with(&root.path))
        .max_by_key(|root| root.path.components().count())
}

pub(super) fn replace_prefix(path: &Path, from: &Path, to: &Path) -> PathBuf {
    match path.strip_prefix(from) {
        Ok(rest) if rest.as_os_str().is_empty() => to.to_owned(),
        Ok(rest) => to.join(rest),
        Err(_) => path.to_owned(),
    }
}

fn rename_prefix(local: &mut BTreeMap<PathBuf, LocalMetadata>, from: &Path, to: &Path) {
    let moved: Vec<PathBuf> = local
        .keys()
        .filter(|p| p.starts_with(from))
        .cloned()
        .collect();
    for path in moved {
        if let Some(meta) = local.remove(&path) {
            local.insert(replace_prefix(&path, from, to), meta);
        }
    }
}

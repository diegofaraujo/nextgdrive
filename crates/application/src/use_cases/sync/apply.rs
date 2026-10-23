//! Carrying out a plan. Each operation that fails is reported and the rest
//! goes on; the next sync retries from whatever state results.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use nextgdrive_domain::local_name::{conflict_copy_name, drive_name, google_doc_shortcut};
use nextgdrive_domain::{Account, DriveItem, DriveItemId, ItemKind};

use super::plan::{Op, replace_prefix};
use super::remote::Wanted;
use super::{ConflictReason, SyncProgress, SyncReport};
use crate::ports::{
    Authenticator, CredentialStore, DriveService, FileWriter, LocalFileSystem, LocalMetadata,
    PortError, SyncStateStore, SyncedItem, SyncedKind, UploadTarget,
};
use crate::use_cases::local_copies::{Removal, remove_local_copy};
use crate::{AppError, TokenProvider};

pub(super) struct Executor<'a, A, C, D, F, T> {
    pub tokens: &'a TokenProvider<A, C>,
    pub drive: &'a D,
    pub fs: &'a F,
    pub store: &'a T,
    pub account: &'a Account,
    /// Absolute account folder; every other path is relative to it.
    pub dir: PathBuf,
    pub conflict_label: String,
    pub remote: BTreeMap<DriveItemId, Wanted>,
    pub state: HashMap<DriveItemId, SyncedItem>,
    /// Drive folder id for each local folder path.
    pub folders: BTreeMap<PathBuf, DriveItemId>,
    pub report: SyncReport,
    pub progress: SyncProgress,
}

impl<A, C, D, F, T> Executor<'_, A, C, D, F, T>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    F: LocalFileSystem,
    T: SyncStateStore,
{
    pub async fn run(
        &mut self,
        ops: Vec<Op>,
        on_progress: &(dyn Fn(&SyncProgress) + Sync),
    ) -> Result<(), AppError> {
        for op in &ops {
            match op {
                Op::Download(id) | Op::DownloadOrAdopt(id) | Op::ConflictCopy { id, .. } => {
                    self.progress.files_total += 1;
                    self.progress.bytes_total +=
                        self.remote.get(id).map_or(0, |w| file_size(&w.item));
                }
                Op::UploadNew { path, .. } | Op::UploadUpdate { path, .. } => {
                    self.progress.files_total += 1;
                    let meta = self.fs.metadata(&self.dir.join(path)).ok().flatten();
                    self.progress.bytes_total += meta.map_or(0, |m| m.size);
                }
                _ => {}
            }
        }
        on_progress(&self.progress);
        for op in ops {
            let counted = matches!(
                op,
                Op::Download(_)
                    | Op::DownloadOrAdopt(_)
                    | Op::ConflictCopy { .. }
                    | Op::UploadNew { .. }
                    | Op::UploadUpdate { .. }
            );
            let label = op_path(&op, &self.remote);
            match self.apply(op).await {
                Ok(()) => {}
                // Bookkeeping failures make the state unreliable: stop.
                Err(e @ AppError::Storage(_)) if label.is_none() => return Err(e),
                Err(e) => {
                    let path = self.dir.join(label.unwrap_or_default());
                    self.report.failures.push((path, e));
                }
            }
            if counted {
                self.progress.files_done += 1;
                on_progress(&self.progress);
            }
        }
        Ok(())
    }

    async fn apply(&mut self, op: Op) -> Result<(), AppError> {
        match op {
            Op::Forget(id) => self.forget(&id),
            Op::Keep { path, reason } => {
                self.report.conflict(self.dir.join(path), reason);
                Ok(())
            }
            Op::RemoveLocal { id, path } => self.remove_local(&id, &path),
            Op::Trash(id) => self.trash(&id).await,
            Op::MoveLocal { id, from, to } => self.move_local(&id, &from, &to),
            Op::MoveRemote { id, to } => self.move_remote(&id, &to).await,
            Op::MakeLocalDir(id) => self.make_local_dir(&id),
            Op::MakeRemoteDir { path, selection } => self.make_remote_dir(&path, &selection).await,
            Op::Download(id) => self.download(&id).await,
            Op::DownloadOrAdopt(id) => self.download_or_adopt(&id).await,
            Op::UploadUpdate { id, path } => {
                let selection = self.wanted(&id)?.selection.clone();
                self.upload(UploadKind::Update(&id), &path, &selection)
                    .await
            }
            Op::UploadNew { path, selection } => {
                self.upload(UploadKind::New, &path, &selection).await
            }
            Op::ConflictCopy { id, path } => self.conflict_copy(&id, &path).await,
            Op::Record(id) => {
                let w = self.wanted(&id)?.clone();
                let meta = self.local_meta(&w.path)?;
                self.record(&w.item, &w.selection, &w.path, &meta)
            }
        }
    }

    fn wanted(&self, id: &DriveItemId) -> Result<&Wanted, AppError> {
        self.remote
            .get(id)
            .ok_or_else(|| AppError::Drive(PortError::new(format!("no longer in Drive: {id}"))))
    }

    fn local_meta(&self, path: &Path) -> Result<LocalMetadata, AppError> {
        self.fs
            .metadata(&self.dir.join(path))
            .map_err(AppError::Storage)?
            .ok_or_else(|| {
                AppError::Storage(PortError::new(format!("{} disappeared", path.display())))
            })
    }

    fn folder_id(&self, path: &Path) -> Result<DriveItemId, AppError> {
        path.parent()
            .and_then(|parent| self.folders.get(parent))
            .cloned()
            .ok_or_else(|| {
                AppError::Drive(PortError::new(format!(
                    "no Drive folder for {}",
                    path.display()
                )))
            })
    }

    fn forget(&mut self, id: &DriveItemId) -> Result<(), AppError> {
        self.state.remove(id);
        self.store
            .remove(&self.account.id, id)
            .map_err(AppError::Storage)
    }

    fn record(
        &mut self,
        item: &DriveItem,
        selection: &DriveItemId,
        path: &Path,
        local: &LocalMetadata,
    ) -> Result<(), AppError> {
        let synced = SyncedItem {
            id: item.id.clone(),
            selection: selection.clone(),
            path: path.to_owned(),
            kind: match item.kind {
                ItemKind::Folder => SyncedKind::Folder,
                ItemKind::GoogleDoc { .. } => SyncedKind::GoogleDoc,
                _ => SyncedKind::File,
            },
            remote_modified: item.modified,
            remote_md5: match &item.kind {
                ItemKind::File { md5, .. } => md5.clone(),
                _ => None,
            },
            local_size: if local.is_dir { 0 } else { local.size },
            local_modified: if local.is_dir {
                item.modified
            } else {
                local.modified
            },
            local_inode: local.inode,
        };
        if self.state.get(&synced.id) != Some(&synced) {
            self.store
                .upsert(&self.account.id, &synced)
                .map_err(AppError::Storage)?;
            self.state.insert(synced.id.clone(), synced);
        }
        Ok(())
    }

    fn remove_local(&mut self, id: &DriveItemId, path: &Path) -> Result<(), AppError> {
        let Some(synced) = self.state.get(id).cloned() else {
            return Ok(());
        };
        let absolute = self.dir.join(path);
        match remove_local_copy(
            self.fs,
            &absolute,
            &SyncedItem {
                path: path.to_owned(),
                ..synced
            },
        ) {
            Ok(Removal::Removed) => self.report.removed += 1,
            Ok(Removal::KeptChanged) => self.report.conflict(absolute, ConflictReason::TypeClash),
            Ok(Removal::KeptNotEmpty | Removal::Absent) => {}
            Err(e) => self.report.failures.push((absolute, AppError::Storage(e))),
        }
        self.forget(id)
    }

    async fn trash(&mut self, id: &DriveItemId) -> Result<(), AppError> {
        let w = self.wanted(id)?.clone();
        let token = self.tokens.access_token(self.account).await?;
        let caps = w.item.capabilities;
        let parent = w.item.parents.first();
        let result = if caps.trash {
            self.drive.trash(&token, id).await
        } else if let (true, Some(parent)) = (caps.remove_from_my_drive, parent) {
            self.drive.remove_from_my_drive(&token, id, parent).await
        } else {
            // Not ours to delete: put the local copy back.
            self.report
                .conflict(self.dir.join(&w.path), ConflictReason::CannotDelete);
            self.forget(id)?;
            return if w.item.kind == ItemKind::Folder {
                self.make_local_dir(id)
            } else {
                self.download(id).await
            };
        };
        result.map_err(AppError::Drive)?;
        self.report.trashed += 1;
        self.forget(id)
    }

    fn move_local(&mut self, id: &DriveItemId, from: &Path, to: &Path) -> Result<(), AppError> {
        if let Some(parent) = self.dir.join(to).parent() {
            self.fs.create_dir_all(parent).map_err(AppError::Storage)?;
        }
        self.fs
            .rename(&self.dir.join(from), &self.dir.join(to))
            .map_err(AppError::Storage)?;
        self.report.moved += 1;
        self.repath(id, from, to)
    }

    /// Records new paths for an item and, for folders, everything inside it.
    fn repath(&mut self, id: &DriveItemId, from: &Path, to: &Path) -> Result<(), AppError> {
        let folder = self
            .state
            .get(id)
            .is_some_and(|s| s.kind == SyncedKind::Folder);
        let ids: Vec<DriveItemId> = self
            .state
            .values()
            .filter(|s| &s.id == id || (folder && s.path.starts_with(from)))
            .map(|s| s.id.clone())
            .collect();
        for moved in ids {
            if let Some(mut synced) = self.state.get(&moved).cloned() {
                synced.path = replace_prefix(&synced.path, from, to);
                if let Ok(Some(meta)) = self.fs.metadata(&self.dir.join(&synced.path)) {
                    synced.local_inode = meta.inode;
                }
                self.store
                    .upsert(&self.account.id, &synced)
                    .map_err(AppError::Storage)?;
                self.state.insert(moved, synced);
            }
        }
        if folder {
            let inside: Vec<PathBuf> = self
                .folders
                .keys()
                .filter(|p| p.starts_with(from))
                .cloned()
                .collect();
            for path in inside {
                if let Some(folder_id) = self.folders.remove(&path) {
                    self.folders
                        .insert(replace_prefix(&path, from, to), folder_id);
                }
            }
        }
        Ok(())
    }

    async fn move_remote(&mut self, id: &DriveItemId, to: &Path) -> Result<(), AppError> {
        let w = self.wanted(id)?.clone();
        let from_path = self
            .state
            .get(id)
            .map(|s| s.path.clone())
            .unwrap_or_else(|| w.path.clone());
        let from_parent = w
            .item
            .parents
            .first()
            .cloned()
            .ok_or_else(|| AppError::Drive(PortError::new(format!("{id} has no parent"))))?;
        let to_parent = self.folder_id(to)?;
        let local_name = to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = drive_name(&local_name, &w.item.kind);
        let token = self.tokens.access_token(self.account).await?;
        let item = self
            .drive
            .move_item(&token, id, &from_parent, &to_parent, &name)
            .await
            .map_err(AppError::Drive)?;
        self.report.moved_in_drive += 1;
        self.repath(id, &from_path, to)?;
        if let Some(w) = self.remote.get_mut(id) {
            w.item = item.clone();
            w.path = to.to_owned();
        }
        if item.kind == ItemKind::Folder {
            self.folders.insert(to.to_owned(), id.clone());
        }
        let meta = self.local_meta(to)?;
        let selection = self.wanted(id)?.selection.clone();
        self.record(&item, &selection, to, &meta)
    }

    fn make_local_dir(&mut self, id: &DriveItemId) -> Result<(), AppError> {
        let w = self.wanted(id)?.clone();
        self.fs
            .create_dir_all(&self.dir.join(&w.path))
            .map_err(AppError::Storage)?;
        self.folders.insert(w.path.clone(), id.clone());
        let meta = self.local_meta(&w.path)?;
        self.record(&w.item, &w.selection, &w.path, &meta)
    }

    async fn make_remote_dir(
        &mut self,
        path: &Path,
        selection: &DriveItemId,
    ) -> Result<(), AppError> {
        let parent = self.folder_id(path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let token = self.tokens.access_token(self.account).await?;
        let item = self
            .drive
            .create_folder(&token, &parent, &name)
            .await
            .map_err(AppError::Drive)?;
        self.folders.insert(path.to_owned(), item.id.clone());
        self.track_remote(&item, path, selection);
        let meta = self.local_meta(path)?;
        self.record(&item, selection, path, &meta)
    }

    /// Makes an item created in Drive during this sync known to later operations.
    fn track_remote(&mut self, item: &DriveItem, path: &Path, selection: &DriveItemId) {
        self.remote.insert(
            item.id.clone(),
            Wanted {
                item: item.clone(),
                path: path.to_owned(),
                selection: selection.clone(),
            },
        );
    }

    async fn download(&mut self, id: &DriveItemId) -> Result<(), AppError> {
        let w = self.wanted(id)?.clone();
        let path = self.dir.join(&w.path);
        if let Some(parent) = path.parent() {
            self.fs.create_dir_all(parent).map_err(AppError::Storage)?;
        }
        let mut writer = self
            .fs
            .begin_write(&path)
            .await
            .map_err(AppError::Storage)?;
        match &w.item.kind {
            ItemKind::GoogleDoc { url } => {
                let shortcut = google_doc_shortcut(&w.item.name, url);
                writer
                    .write(shortcut.as_bytes())
                    .await
                    .map_err(AppError::Storage)?;
            }
            ItemKind::File { md5, .. } => {
                let token = self.tokens.access_token(self.account).await?;
                self.drive
                    .download(&token, id, &mut writer)
                    .await
                    .map_err(AppError::Drive)?;
                if let Some(expected) = md5
                    && writer.md5_hex() != *expected
                {
                    return Err(AppError::Drive(PortError::new(
                        "downloaded content does not match its checksum",
                    )));
                }
            }
            _ => return Err(AppError::NotAFolder(id.clone())),
        }
        let meta = self
            .fs
            .commit(writer, w.item.modified)
            .await
            .map_err(AppError::Storage)?;
        self.progress.bytes_done += meta.size;
        self.report.downloaded += 1;
        self.report.downloaded_bytes += meta.size;
        self.record(&w.item, &w.selection, &w.path, &meta)
    }

    async fn download_or_adopt(&mut self, id: &DriveItemId) -> Result<(), AppError> {
        let w = self.wanted(id)?.clone();
        let path = self.dir.join(&w.path);
        let identical = match &w.item.kind {
            ItemKind::File { md5: Some(md5), .. } => {
                self.fs.md5_hex(&path).await.map_err(AppError::Storage)? == *md5
            }
            // Shortcuts are generated, so the expected content is known exactly.
            ItemKind::GoogleDoc { url } => {
                let shortcut = google_doc_shortcut(&w.item.name, url);
                self.fs
                    .has_content(&path, shortcut.as_bytes())
                    .await
                    .map_err(AppError::Storage)?
            }
            _ => false,
        };
        if identical {
            let meta = self.local_meta(&w.path)?;
            return self.record(&w.item, &w.selection, &w.path, &meta);
        }
        self.conflict_copy(id, &w.path).await
    }

    /// Keeps the local version under a new name and uploads it, then puts
    /// Drive's version at its own path.
    async fn conflict_copy(&mut self, id: &DriveItemId, path: &Path) -> Result<(), AppError> {
        let w = self.wanted(id)?.clone();
        let copy = self.free_copy_name(path)?;
        self.fs
            .rename(&self.dir.join(path), &self.dir.join(&copy))
            .map_err(AppError::Storage)?;
        self.state.remove(id);
        self.report.conflict_copies.push(self.dir.join(&copy));
        self.upload(UploadKind::New, &copy, &w.selection).await?;
        self.download(id).await
    }

    fn free_copy_name(&self, path: &Path) -> Result<PathBuf, AppError> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for n in 1.. {
            let label = if n == 1 {
                self.conflict_label.clone()
            } else {
                format!("{} {n}", self.conflict_label)
            };
            let candidate = path.with_file_name(conflict_copy_name(&name, &label));
            if self
                .fs
                .metadata(&self.dir.join(&candidate))
                .map_err(AppError::Storage)?
                .is_none()
            {
                return Ok(candidate);
            }
        }
        unreachable!("an unused name always exists")
    }

    async fn upload(
        &mut self,
        kind: UploadKind<'_>,
        path: &Path,
        selection: &DriveItemId,
    ) -> Result<(), AppError> {
        // Taken before reading: if the file changes during the upload, its
        // new mtime marks it as changed for the next sync.
        let meta = self.local_meta(path)?;
        let mut reader = self
            .fs
            .open_read(&self.dir.join(path))
            .await
            .map_err(AppError::Storage)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = match kind {
            UploadKind::New => Some(self.folder_id(path)?),
            UploadKind::Update(_) => None,
        };
        let target = match (kind, &parent) {
            (UploadKind::Update(id), _) => UploadTarget::Existing(id),
            (UploadKind::New, Some(parent)) => UploadTarget::New {
                parent,
                name: &name,
            },
            (UploadKind::New, None) => unreachable!("new uploads have a parent"),
        };
        let token = self.tokens.access_token(self.account).await?;
        let item = self
            .drive
            .upload(&token, target, meta.size, meta.modified, &mut reader)
            .await
            .map_err(AppError::Drive)?;
        self.progress.bytes_done += meta.size;
        self.report.uploaded += 1;
        self.report.uploaded_bytes += meta.size;
        self.track_remote(&item, path, selection);
        self.record(&item, selection, path, &meta)
    }
}

#[derive(Clone, Copy)]
enum UploadKind<'a> {
    New,
    Update(&'a DriveItemId),
}

/// Path an operation is about, for failure reports.
fn op_path(op: &Op, remote: &BTreeMap<DriveItemId, Wanted>) -> Option<PathBuf> {
    let of = |id: &DriveItemId| remote.get(id).map(|w| w.path.clone());
    match op {
        Op::Forget(_) => None,
        Op::RemoveLocal { path, .. }
        | Op::MakeRemoteDir { path, .. }
        | Op::UploadUpdate { path, .. }
        | Op::UploadNew { path, .. }
        | Op::ConflictCopy { path, .. }
        | Op::Keep { path, .. } => Some(path.clone()),
        Op::MoveLocal { to, .. } | Op::MoveRemote { to, .. } => Some(to.clone()),
        Op::Trash(id)
        | Op::MakeLocalDir(id)
        | Op::Download(id)
        | Op::DownloadOrAdopt(id)
        | Op::Record(id) => of(id),
    }
}

fn file_size(item: &DriveItem) -> u64 {
    match item.kind {
        ItemKind::File { size, .. } => size,
        _ => 0,
    }
}

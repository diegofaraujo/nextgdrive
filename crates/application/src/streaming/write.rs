//! Writing through the streaming tree.
//!
//! Rules, the same as two-way sync's:
//! - Written content goes to the staging area first and stays there until it
//!   is in Drive; a failed upload is retried later, also after a restart.
//! - If the file changed in Drive since editing began, the edit is uploaded
//!   as a conflict copy next to it instead of overwriting.
//! - Deleting trashes (or, for others' items, removes from My Drive).
//! - Editors' temporary files are never uploaded; renaming one over a file
//!   uploads its content as a new version of that file.

use std::time::{Instant, SystemTime};

use nextgdrive_domain::local_name::{conflict_copy_name, drive_name, is_ignored};
use nextgdrive_domain::{DriveItem, DriveItemId, ItemKind};

use super::{
    Content, Entry, Node, NodeId, Staging, StreamError, StreamingDrive, content_of, version_of,
};
use crate::AppError;
use crate::ports::{
    Authenticator, BlockCache, BlockKey, CredentialStore, DriveService, FileReader, PortError,
    SettingsRepository, StagedFile, StagedTarget, StagingArea, UploadTarget,
};

/// What uploading a staged file did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadOutcome {
    /// Name of the conflict copy created because the file had changed in Drive.
    pub conflict_copy: Option<String>,
}

/// Reads a staged file for uploading.
struct StagedReader<'a, G> {
    staging: &'a G,
    key: String,
    offset: u64,
}

impl<G: StagingArea> FileReader for StagedReader<'_, G> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, PortError> {
        let data = self.staging.read_at(&self.key, self.offset, buf.len())?;
        buf[..data.len()].copy_from_slice(&data);
        self.offset += data.len() as u64;
        Ok(data.len())
    }
}

impl<A, C, D, K, S, G> StreamingDrive<'_, A, C, D, K, S, G>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    K: BlockCache,
    S: SettingsRepository,
    G: StagingArea,
{
    /// Picks up files staged before a restart, so they are shown and uploaded.
    pub fn recover(&self) -> Result<usize, AppError> {
        let files = self.staging.list().map_err(AppError::Storage)?;
        let mut lens = Vec::with_capacity(files.len());
        for file in &files {
            lens.push(self.staging.len(&file.key).map_err(AppError::Storage)?);
        }
        let mut table = self.state.table();
        for (file, len) in files.iter().zip(lens) {
            if let StagedTarget::Existing { id, .. } = &file.target {
                table.staged_by_drive.insert(id.clone(), file.key.clone());
            }
            table.staged.insert(
                file.key.clone(),
                Staging {
                    file: file.clone(),
                    len,
                    // Not known to be in Drive: upload.
                    generation: 1,
                    uploaded: 0,
                    changed: Instant::now(),
                    retry_after: None,
                },
            );
        }
        table.listings.clear();
        Ok(files.len())
    }

    /// Uploads staged files with changes not in Drive yet: edits of existing
    /// files, and new files left unchanged for `new_file_quiet`. Files whose
    /// last upload failed wait for `retry_backoff`.
    pub async fn upload_pending(&self) -> Vec<(String, Result<UploadOutcome, AppError>)> {
        let config = self.state.config;
        let keys: Vec<(String, String)> = {
            let table = self.state.table();
            table
                .staged
                .values()
                .filter(|s| s.dirty() && !is_ignored(&s.file.name))
                .filter(|s| s.retry_after.is_none_or(|at| Instant::now() >= at))
                .filter(|s| match s.file.target {
                    StagedTarget::Existing { .. } => true,
                    StagedTarget::New { .. } => s.changed.elapsed() >= config.new_file_quiet,
                })
                .map(|s| (s.file.key.clone(), s.file.name.clone()))
                .collect()
        };
        let mut results = Vec::new();
        for (key, name) in keys {
            results.push((name, self.upload(&key).await));
        }
        results
    }

    pub async fn create(&self, dir: NodeId, name: &str) -> Result<Entry, StreamError> {
        let parent = self.folder_of(dir)?;
        let file = StagedFile {
            key: self.new_key(),
            name: name.to_owned(),
            target: StagedTarget::New { parent },
            modified: SystemTime::now(),
        };
        self.staging.create(&file).map_err(AppError::Storage)?;
        let mut table = self.state.table();
        let id = table.node_for(dir, format!("staged:{}", file.key));
        table.nodes.insert(
            id,
            Node {
                content: Content::Staged(file.key.clone()),
                modified: file.modified,
            },
        );
        table.staged.insert(
            file.key.clone(),
            Staging {
                file,
                len: 0,
                // Even an empty new file is uploaded.
                generation: 1,
                uploaded: 0,
                changed: Instant::now(),
                retry_after: None,
            },
        );
        table.invalidate(dir);
        let attr = table.attr(id).expect("just added");
        Ok(Entry {
            name: name.to_owned(),
            attr,
        })
    }

    /// Prepares a file for writing: its content moves to the staging area
    /// (none of it if `truncate`).
    pub async fn open_write(&self, node: NodeId, truncate: bool) -> Result<(), StreamError> {
        let content = self
            .state
            .table()
            .content(node)
            .ok_or(StreamError::NotFound)?;
        match content {
            Content::Staged(key) => {
                if truncate {
                    self.resize(&key, 0)?;
                }
                Ok(())
            }
            Content::File { id, size, version } => {
                let token = self.tokens.access_token(self.account).await?;
                let item = self
                    .drive
                    .item(&token, &id)
                    .await
                    .map_err(AppError::Drive)?;
                let file = StagedFile {
                    key: self.new_key(),
                    name: item.name,
                    target: StagedTarget::Existing {
                        id: id.clone(),
                        base_version: version.clone(),
                    },
                    modified: item.modified,
                };
                self.staging.create(&file).map_err(AppError::Storage)?;
                let len = if truncate { 0 } else { size };
                if !truncate {
                    // Copied in large pieces, through the block cache.
                    let piece =
                        self.state.config.block_size * self.state.config.readahead_blocks.max(1);
                    let mut offset = 0;
                    while offset < size {
                        let end = (offset + piece).min(size);
                        let data = self
                            .read_range(
                                &id,
                                &version,
                                size,
                                offset,
                                end,
                                self.state.config.readahead_blocks,
                            )
                            .await?;
                        self.staging
                            .write_at(&file.key, offset, &data)
                            .map_err(AppError::Storage)?;
                        offset = end;
                    }
                }
                let mut table = self.state.table();
                table.staged_by_drive.insert(id, file.key.clone());
                if let Some(node) = table.nodes.get_mut(&node) {
                    node.content = Content::Staged(file.key.clone());
                }
                table.staged.insert(
                    file.key.clone(),
                    Staging {
                        file,
                        len,
                        generation: u64::from(truncate),
                        uploaded: 0,
                        changed: Instant::now(),
                        retry_after: None,
                    },
                );
                Ok(())
            }
            Content::Shortcut { .. } => Err(StreamError::NotPermitted),
            _ => Err(StreamError::IsDir),
        }
    }

    /// Writes into a file opened with [`open_write`](Self::open_write).
    pub fn write(&self, node: NodeId, offset: u64, data: &[u8]) -> Result<u32, StreamError> {
        let key = self.staged_key(node)?;
        self.staging
            .write_at(&key, offset, data)
            .map_err(AppError::Storage)?;
        let mut table = self.state.table();
        if let Some(staging) = table.staged.get_mut(&key) {
            staging.len = staging.len.max(offset + data.len() as u64);
            staging.generation += 1;
            staging.changed = Instant::now();
            staging.file.modified = SystemTime::now();
        }
        Ok(u32::try_from(data.len()).unwrap_or(u32::MAX))
    }

    pub async fn set_len(&self, node: NodeId, len: u64) -> Result<(), StreamError> {
        if self.staged_key(node).is_err() {
            self.open_write(node, len == 0).await?;
        }
        let key = self.staged_key(node)?;
        self.resize(&key, len)
    }

    /// Called when a program closes the file: uploads edits of existing files.
    pub async fn flush(&self, node: NodeId) -> Result<Option<UploadOutcome>, StreamError> {
        let Ok(key) = self.staged_key(node) else {
            return Ok(None);
        };
        // New files wait for `upload_pending` (see `new_file_quiet`).
        let edit =
            self.state.table().staged.get(&key).is_some_and(|s| {
                s.dirty() && matches!(s.file.target, StagedTarget::Existing { .. })
            });
        if !edit {
            return Ok(None);
        }
        Ok(Some(self.upload(&key).await?))
    }

    pub async fn mkdir(&self, dir: NodeId, name: &str) -> Result<Entry, StreamError> {
        let parent = self.folder_of(dir)?;
        let token = self.tokens.access_token(self.account).await?;
        let item = self
            .drive
            .create_folder(&token, &parent, name)
            .await
            .map_err(AppError::Drive)?;
        let mut table = self.state.table();
        let id = table.node_for(dir, item.id.to_string());
        table.nodes.insert(
            id,
            Node {
                content: Content::Folder(item.id),
                modified: item.modified,
            },
        );
        table.invalidate(dir);
        let attr = table.attr(id).expect("just added");
        Ok(Entry {
            name: name.to_owned(),
            attr,
        })
    }

    pub async fn unlink(&self, dir: NodeId, name: &str) -> Result<(), StreamError> {
        let entry = self.lookup(dir, name).await?.ok_or(StreamError::NotFound)?;
        let content = self
            .state
            .table()
            .content(entry.attr.id)
            .ok_or(StreamError::NotFound)?;
        match content {
            Content::Staged(key) => {
                let target = self
                    .state
                    .table()
                    .staged
                    .get(&key)
                    .map(|s| s.file.target.clone());
                if let Some(StagedTarget::Existing { id, .. }) = target {
                    self.trash_item(&id, &self.folder_of(dir)?).await?;
                }
                self.drop_staged(&key)?;
            }
            Content::File { id, .. } | Content::Shortcut { id, .. } => {
                self.trash_item(&id, &self.folder_of(dir)?).await?;
            }
            _ => return Err(StreamError::IsDir),
        }
        self.state.table().invalidate(dir);
        Ok(())
    }

    pub async fn rmdir(&self, dir: NodeId, name: &str) -> Result<(), StreamError> {
        let entry = self.lookup(dir, name).await?.ok_or(StreamError::NotFound)?;
        let content = self
            .state
            .table()
            .content(entry.attr.id)
            .ok_or(StreamError::NotFound)?;
        match content {
            Content::Folder(id) => {
                if !self.list(entry.attr.id).await?.is_empty() {
                    return Err(StreamError::NotEmpty);
                }
                self.trash_item(&id, &self.folder_of(dir)?).await?;
            }
            Content::Root | Content::SharedDrives => return Err(StreamError::NotPermitted),
            _ => return Err(StreamError::NotDir),
        }
        self.state.table().invalidate(dir);
        Ok(())
    }

    pub async fn rename(
        &self,
        dir: NodeId,
        name: &str,
        new_dir: NodeId,
        new_name: &str,
    ) -> Result<(), StreamError> {
        let source = self.lookup(dir, name).await?.ok_or(StreamError::NotFound)?;
        let old_parent = self.folder_of(dir)?;
        let new_parent = self.folder_of(new_dir)?;
        let target = self
            .lookup(new_dir, new_name)
            .await?
            .filter(|t| t.attr.id != source.attr.id);
        let content = self
            .state
            .table()
            .content(source.attr.id)
            .ok_or(StreamError::NotFound)?;

        // Values are taken out of the table first: a lock in an `if let` or
        // `match` condition lasts for the whole block, and locking again
        // inside it would deadlock.
        let source_target = match &content {
            Content::Staged(key) => self.staged_target(key),
            _ => None,
        };
        let target_content = target
            .as_ref()
            .and_then(|t| self.state.table().content(t.attr.id));

        // A file never uploaded (typically an editor's temporary file).
        if let Content::Staged(key) = &content
            && let Some(StagedTarget::New { .. }) = source_target
        {
            let new_target = match &target {
                None => StagedTarget::New {
                    parent: new_parent.clone(),
                },
                Some(_) => match target_content {
                    // Saving over a file: its new version, keeping history and sharing.
                    Some(Content::File { id, version, .. }) => StagedTarget::Existing {
                        id,
                        base_version: version,
                    },
                    Some(Content::Staged(other)) => {
                        let replaced = self.staged_target(&other);
                        self.drop_staged(&other)?;
                        replaced.unwrap_or(StagedTarget::New {
                            parent: new_parent.clone(),
                        })
                    }
                    _ => return Err(StreamError::NotPermitted),
                },
            };
            let file = {
                let mut table = self.state.table();
                let staging = table.staged.get_mut(key).ok_or(StreamError::NotFound)?;
                staging.file.name = new_name.to_owned();
                staging.file.target = new_target.clone();
                let file = staging.file.clone();
                if let StagedTarget::Existing { id, .. } = &new_target {
                    table.staged_by_drive.insert(id.clone(), key.clone());
                    table
                        .by_key
                        .insert((new_dir, id.to_string()), source.attr.id);
                } else {
                    table
                        .by_key
                        .insert((new_dir, format!("staged:{key}")), source.attr.id);
                }
                table.invalidate(dir);
                table.invalidate(new_dir);
                file
            };
            self.staging.update(&file).map_err(AppError::Storage)?;
            // Renamed after it was closed: no flush will come. A failure is
            // retried later; the content is safe in the staging area.
            if !is_ignored(new_name) {
                let _ = self.upload(key).await;
            }
            return Ok(());
        }

        let (drive_id, kind) = match &content {
            Content::File { id, .. } => (id.clone(), ItemKind::File { size: 0, md5: None }),
            Content::Shortcut { id, .. } => {
                (id.clone(), ItemKind::GoogleDoc { url: String::new() })
            }
            Content::Folder(id) => (id.clone(), ItemKind::Folder),
            Content::Staged(_) => match source_target {
                Some(StagedTarget::Existing { id, .. }) => {
                    (id, ItemKind::File { size: 0, md5: None })
                }
                _ => return Err(StreamError::NotFound),
            },
            Content::Root | Content::SharedDrives => return Err(StreamError::NotPermitted),
        };
        // Replacing: what was there goes to the trash, where it can be restored.
        if let Some(target) = target {
            match target.attr.kind {
                super::NodeKind::File => self.unlink(new_dir, new_name).await?,
                super::NodeKind::Dir if matches!(kind, ItemKind::Folder) => {
                    self.rmdir(new_dir, new_name).await?
                }
                super::NodeKind::Dir => return Err(StreamError::IsDir),
            }
        }
        let name_in_drive = drive_name(new_name, &kind);
        let token = self.tokens.access_token(self.account).await?;
        self.drive
            .move_item(&token, &drive_id, &old_parent, &new_parent, &name_in_drive)
            .await
            .map_err(AppError::Drive)?;
        let staged_file = {
            let mut table = self.state.table();
            table
                .by_key
                .insert((new_dir, drive_id.to_string()), source.attr.id);
            table.invalidate(dir);
            table.invalidate(new_dir);
            match &content {
                Content::Staged(key) => table.staged.get_mut(key).map(|s| {
                    s.file.name = name_in_drive.clone();
                    s.file.clone()
                }),
                _ => None,
            }
        };
        if let Some(file) = staged_file {
            self.staging.update(&file).map_err(AppError::Storage)?;
        }
        Ok(())
    }

    async fn upload(&self, key: &str) -> Result<UploadOutcome, AppError> {
        let result = self.try_upload(key).await;
        if result.is_err() {
            let mut table = self.state.table();
            if let Some(staging) = table.staged.get_mut(key) {
                staging.retry_after = Some(Instant::now() + self.state.config.retry_backoff);
            }
        }
        result
    }

    async fn try_upload(&self, key: &str) -> Result<UploadOutcome, AppError> {
        let Some((file, generation)) = self
            .state
            .table()
            .staged
            .get(key)
            .filter(|s| s.dirty())
            .map(|s| (s.file.clone(), s.generation))
        else {
            return Ok(UploadOutcome::default());
        };
        if is_ignored(&file.name) {
            return Ok(UploadOutcome::default());
        }
        let token = self.tokens.access_token(self.account).await?;
        let len = self.staging.len(key).map_err(AppError::Storage)?;
        // (parent and name of a new file, or the file to update), and what
        // the original file is now if this became a conflict copy.
        let (new_file, existing, current) = match &file.target {
            StagedTarget::Existing { id, base_version } => {
                let current = self.drive.item(&token, id).await.map_err(AppError::Drive)?;
                if version_of(&current) == *base_version {
                    (None, Some(id.clone()), None)
                } else {
                    let parent = current
                        .parents
                        .first()
                        .cloned()
                        .ok_or_else(|| AppError::Drive(PortError::new("file has no folder")))?;
                    let name = conflict_copy_name(&file.name, &self.conflict_label);
                    (Some((parent, name)), None, Some(current))
                }
            }
            StagedTarget::New { parent } => (Some((parent.clone(), file.name.clone())), None, None),
        };
        let target = match (&new_file, &existing) {
            (Some((parent, name)), _) => UploadTarget::New { parent, name },
            (None, Some(id)) => UploadTarget::Existing(id),
            (None, None) => unreachable!("one of them is set"),
        };
        let mut reader = StagedReader {
            staging: self.staging,
            key: key.to_owned(),
            offset: 0,
        };
        let item = self
            .drive
            .upload(&token, target, len, file.modified, &mut reader)
            .await
            .map_err(AppError::Drive)?;
        // Deleted while uploading: what was just created must not stay.
        // (For an edit of an existing file, deleting already trashed it.)
        let deleted = !self.state.table().staged.contains_key(key);
        if deleted {
            if new_file.is_some() {
                self.drive
                    .trash(&token, &item.id)
                    .await
                    .map_err(AppError::Drive)?;
            }
            return Ok(UploadOutcome::default());
        }
        let outcome = UploadOutcome {
            conflict_copy: current.as_ref().and(new_file.map(|(_, name)| name)),
        };
        self.finish_upload(key, generation, &item, current.as_ref(), len)?;
        Ok(outcome)
    }

    /// Records an upload. `current` is the original file when the upload
    /// became a conflict copy.
    fn finish_upload(
        &self,
        key: &str,
        generation: u64,
        item: &DriveItem,
        current: Option<&DriveItem>,
        len: u64,
    ) -> Result<(), AppError> {
        if current.is_none() {
            self.cache_staged(key, item, len);
        }
        let mut table = self.state.table();
        let Some(staging) = table.staged.get_mut(key) else {
            return Ok(()); // Deleted meanwhile.
        };
        if staging.generation != generation {
            // Written to during the upload: keep it staged, as an edit of what
            // was just uploaded.
            staging.uploaded = generation;
            staging.file.target = StagedTarget::Existing {
                id: item.id.clone(),
                base_version: version_of(item),
            };
            let file = staging.file.clone();
            drop(table);
            return self.staging.update(&file).map_err(AppError::Storage);
        }
        let staging = table.staged.remove(key).expect("checked above");
        if let StagedTarget::Existing { id, .. } = &staging.file.target {
            table.staged_by_drive.remove(id);
        }
        if let Some(node) = table.node_of_staged(key) {
            let shown = current.unwrap_or(item);
            if let Some(content) = content_of(shown) {
                table.nodes.insert(
                    node,
                    Node {
                        content,
                        modified: shown.modified,
                    },
                );
            }
            let dirs: Vec<NodeId> = table
                .by_key
                .iter()
                .filter(|(_, n)| **n == node)
                .map(|((dir, _), _)| *dir)
                .collect();
            for dir in dirs {
                table.by_key.insert((dir, shown.id.to_string()), node);
                table.invalidate(dir);
            }
        }
        drop(table);
        self.staging.remove(key).map_err(AppError::Storage)
    }

    /// Puts uploaded content in the block cache, so reading it back does not
    /// download it again. Best effort.
    fn cache_staged(&self, key: &str, item: &DriveItem, len: u64) {
        let block_size = self.state.config.block_size;
        let version = version_of(item);
        for index in 0..len.div_ceil(block_size) {
            let Ok(data) = self
                .staging
                .read_at(key, index * block_size, block_size as usize)
            else {
                return;
            };
            let block = BlockKey {
                file: item.id.clone(),
                version: version.clone(),
                index,
            };
            if self.cache.put(&block, &data).is_err() {
                return;
            }
        }
    }

    async fn trash_item(&self, id: &DriveItemId, parent: &DriveItemId) -> Result<(), StreamError> {
        let token = self.tokens.access_token(self.account).await?;
        let item = self.drive.item(&token, id).await.map_err(AppError::Drive)?;
        if item.capabilities.trash {
            self.drive
                .trash(&token, id)
                .await
                .map_err(AppError::Drive)?;
        } else if item.capabilities.remove_from_my_drive {
            self.drive
                .remove_from_my_drive(&token, id, parent)
                .await
                .map_err(AppError::Drive)?;
        } else {
            return Err(StreamError::NotPermitted);
        }
        Ok(())
    }

    fn drop_staged(&self, key: &str) -> Result<(), StreamError> {
        {
            let mut table = self.state.table();
            if let Some(staging) = table.staged.remove(key)
                && let StagedTarget::Existing { id, .. } = staging.file.target
            {
                table.staged_by_drive.remove(&id);
            }
        }
        self.staging.remove(key).map_err(AppError::Storage)?;
        Ok(())
    }

    fn resize(&self, key: &str, len: u64) -> Result<(), StreamError> {
        self.staging.set_len(key, len).map_err(AppError::Storage)?;
        let mut table = self.state.table();
        if let Some(staging) = table.staged.get_mut(key) {
            staging.len = len;
            staging.generation += 1;
            staging.changed = Instant::now();
            staging.file.modified = SystemTime::now();
        }
        Ok(())
    }

    fn staged_target(&self, key: &str) -> Option<StagedTarget> {
        let table = self.state.table();
        table.staged.get(key).map(|s| s.file.target.clone())
    }

    fn staged_key(&self, node: NodeId) -> Result<String, StreamError> {
        match self.state.table().content(node) {
            Some(Content::Staged(key)) => Ok(key),
            Some(_) => Err(StreamError::NotPermitted),
            None => Err(StreamError::NotFound),
        }
    }

    /// The Drive folder behind a node; only real folders can hold files.
    fn folder_of(&self, dir: NodeId) -> Result<DriveItemId, StreamError> {
        match self.state.table().content(dir) {
            Some(Content::Folder(id)) => Ok(id),
            Some(Content::Root | Content::SharedDrives) => Err(StreamError::NotPermitted),
            Some(_) => Err(StreamError::NotDir),
            None => Err(StreamError::NotFound),
        }
    }

    /// Unique across restarts: the mount time plus a counter.
    fn new_key(&self) -> String {
        let mounted = self
            .state
            .mounted_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut table = self.state.table();
        let n = table.next;
        table.next += 1;
        format!("{mounted:x}-{n}")
    }
}

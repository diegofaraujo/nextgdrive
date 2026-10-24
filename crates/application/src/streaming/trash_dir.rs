//! The desktop trash inside a streamed Drive.
//!
//! File managers do not delete: they move to the trash, which on a mount
//! other than home is `<mount>/.Trash-<uid>/` with `files/` (the items) and
//! `info/` (a `.trashinfo` description of each). Here that folder exists only
//! on this computer, and moving something into `files/` moves it to the Drive
//! trash, so "move to trash" behaves as in Drive. Restoring from the desktop
//! trash restores from the Drive trash. Emptying the desktop trash only
//! forgets the entries: Drive deletes trashed items for good after 30 days,
//! never Next GDrive.

use std::time::SystemTime;

use nextgdrive_domain::ItemKind;
use nextgdrive_domain::local_name::drive_name;

use super::{Content, Entry, LocalDir, Node, NodeId, ROOT, StreamError, StreamingDrive};
use crate::AppError;
use crate::ports::{
    Authenticator, BlockCache, CredentialStore, DriveService, SettingsRepository, StagedTarget,
    StagingArea,
};

/// `.Trash-<uid>` (or `.Trash`), as the freedesktop.org trash specification names it.
pub(super) fn is_trash_name(name: &str) -> bool {
    name == ".Trash"
        || name
            .strip_prefix(".Trash-")
            .is_some_and(|uid| !uid.is_empty() && uid.chars().all(|c| c.is_ascii_digit()))
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
    pub(super) fn local_dir(&self, node: NodeId) -> Option<LocalDir> {
        match self.state.table().content(node) {
            Some(Content::Local(kind)) => Some(kind),
            _ => None,
        }
    }

    /// The trash folder and its two subfolders; `None` for other names.
    pub(super) fn mkdir_local(
        &self,
        dir: NodeId,
        name: &str,
    ) -> Option<Result<Entry, StreamError>> {
        let kind = match (dir, self.local_dir(dir)) {
            (ROOT, None) if is_trash_name(name) => LocalDir::Trash,
            (_, Some(LocalDir::Trash)) if name == "files" => LocalDir::TrashFiles,
            (_, Some(LocalDir::Trash)) if name == "info" => LocalDir::TrashInfo,
            (_, Some(_)) => return Some(Err(StreamError::NotPermitted)),
            _ => return None,
        };
        Some(Ok(self.add_local(dir, name, Content::Local(kind))))
    }

    /// A `.trashinfo` description, held in memory.
    pub(super) fn create_local(
        &self,
        dir: NodeId,
        name: &str,
    ) -> Option<Result<Entry, StreamError>> {
        match self.local_dir(dir)? {
            LocalDir::TrashInfo => Some(Ok(self.add_local(
                dir,
                name,
                Content::TrashInfo(Vec::new()),
            ))),
            _ => Some(Err(StreamError::NotPermitted)),
        }
    }

    /// Writes into a `.trashinfo`; `None` if the node is not one.
    pub(super) fn write_local(&self, node: NodeId, offset: u64, data: &[u8]) -> Option<u32> {
        let mut table = self.state.table();
        let Some(Content::TrashInfo(bytes)) = table.nodes.get_mut(&node).map(|n| &mut n.content)
        else {
            return None;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if bytes.len() < start + data.len() {
            bytes.resize(start + data.len(), 0);
        }
        bytes[start..start + data.len()].copy_from_slice(data);
        Some(u32::try_from(data.len()).unwrap_or(u32::MAX))
    }

    /// Resizes a `.trashinfo` (`u64::MAX`: leave as is); `false` if the
    /// node is not one.
    pub(super) fn resize_local(&self, node: NodeId, len: u64) -> bool {
        let mut table = self.state.table();
        match table.nodes.get_mut(&node).map(|n| &mut n.content) {
            Some(Content::TrashInfo(bytes)) => {
                if len != u64::MAX {
                    bytes.resize(usize::try_from(len).unwrap_or(usize::MAX), 0);
                }
                true
            }
            _ => false,
        }
    }

    /// Removes an entry of the desktop trash (emptying it, or a deleted
    /// description). Never deletes anything in Drive.
    pub(super) fn remove_local(&self, dir: NodeId, name: &str) -> bool {
        let mut table = self.state.table();
        let Some(children) = table.local_children.get_mut(&dir) else {
            return false;
        };
        let before = children.len();
        let mut removed = Vec::new();
        children.retain(|(child, id)| {
            let keep = child != name;
            if !keep {
                removed.push(*id);
            }
            keep
        });
        let found = children.len() != before;
        // Whatever was inside goes too.
        while let Some(id) = removed.pop() {
            if let Some(inner) = table.local_children.remove(&id) {
                removed.extend(inner.into_iter().map(|(_, id)| id));
            }
            table.nodes.remove(&id);
        }
        found
    }

    /// Moving something into the desktop trash's `files/`: to the Drive trash.
    pub(super) async fn move_to_trash(
        &self,
        dir: NodeId,
        name: &str,
        trash_files: NodeId,
        new_name: &str,
    ) -> Result<(), StreamError> {
        let source = self.lookup(dir, name).await?.ok_or(StreamError::NotFound)?;
        let content = self
            .state
            .table()
            .content(source.attr.id)
            .ok_or(StreamError::NotFound)?;
        let parent = self.folder_of(dir)?;
        let (id, is_dir) = match content {
            Content::File { id, .. } | Content::Shortcut { id, .. } => (id, false),
            Content::Folder(id) => (id, true),
            Content::Staged(key) => {
                let target = self
                    .state
                    .table()
                    .staged
                    .get(&key)
                    .map(|s| s.file.target.clone());
                self.drop_staged(&key)?;
                match target {
                    Some(StagedTarget::Existing { id, .. }) => (id, false),
                    // Never uploaded: nothing in Drive to trash.
                    _ => {
                        self.state.table().invalidate(dir);
                        return Ok(());
                    }
                }
            }
            _ => return Err(StreamError::NotPermitted),
        };
        self.trash_item(&id, &parent).await?;
        let mut table = self.state.table();
        table.invalidate(dir);
        // Same node: the kernel keeps the moved file's inode under its new name.
        put_local(
            &mut table,
            trash_files,
            new_name,
            source.attr.id,
            Content::Trashed { id, dir: is_dir },
        );
        Ok(())
    }

    /// Moving something out of the desktop trash: restored from the Drive
    /// trash, then moved if it goes elsewhere than where it was.
    pub(super) async fn restore_from_trash(
        &self,
        dir: NodeId,
        name: &str,
        new_dir: NodeId,
        new_name: &str,
    ) -> Result<(), StreamError> {
        let entry = {
            let table = self.state.table();
            table
                .local_children
                .get(&dir)
                .and_then(|c| c.iter().find(|(n, _)| n == name).map(|(_, id)| *id))
                .and_then(|node| Some((node, table.content(node)?)))
        };
        let Some((node, Content::Trashed { id, dir: is_dir })) = entry else {
            return Err(StreamError::NotPermitted);
        };
        let new_parent = self.folder_of(new_dir)?;
        let token = self.tokens.access_token(self.account).await?;
        let item = self
            .drive
            .restore(&token, &id)
            .await
            .map_err(AppError::Drive)?;
        let kind = if is_dir {
            ItemKind::Folder
        } else {
            item.kind.clone()
        };
        let wanted_name = drive_name(new_name, &kind);
        let old_parent = item.parents.first().cloned();
        let item = if old_parent.as_ref() != Some(&new_parent) || item.name != wanted_name {
            let from = old_parent.unwrap_or_else(|| new_parent.clone());
            self.drive
                .move_item(&token, &id, &from, &new_parent, &wanted_name)
                .await
                .map_err(AppError::Drive)?
        } else {
            item
        };
        let mut table = self.state.table();
        if let Some(children) = table.local_children.get_mut(&dir) {
            children.retain(|(n, _)| n != name);
        }
        // Same node again, now the restored item.
        if let Some(content) = super::content_of(&item) {
            table.nodes.insert(
                node,
                Node {
                    content,
                    modified: item.modified,
                },
            );
        }
        table.by_key.insert((new_dir, id.to_string()), node);
        table.invalidate(new_dir);
        Ok(())
    }

    /// Renames an entry within the desktop trash, replacing any of that name.
    pub(super) fn rename_local(
        &self,
        dir: NodeId,
        name: &str,
        new_dir: NodeId,
        new_name: &str,
    ) -> Result<(), StreamError> {
        let mut table = self.state.table();
        let children = table
            .local_children
            .get_mut(&dir)
            .ok_or(StreamError::NotFound)?;
        let index = children
            .iter()
            .position(|(n, _)| n == name)
            .ok_or(StreamError::NotFound)?;
        let (_, id) = children.remove(index);
        let target = table.local_children.entry(new_dir).or_default();
        target.retain(|(n, _)| n != new_name);
        target.push((new_name.to_owned(), id));
        Ok(())
    }

    fn add_local(&self, dir: NodeId, name: &str, content: Content) -> Entry {
        let mut table = self.state.table();
        let id = table.next;
        table.next += 1;
        put_local(&mut table, dir, name, id, content);
        let attr = table.attr(id).expect("just added");
        Entry {
            name: name.to_owned(),
            attr,
        }
    }
}

/// Puts node `id` with `content` in a local folder, replacing any entry of that name.
fn put_local(table: &mut super::Table, dir: NodeId, name: &str, id: NodeId, content: Content) {
    table.nodes.insert(
        id,
        Node {
            content,
            modified: SystemTime::now(),
        },
    );
    let children = table.local_children.entry(dir).or_default();
    children.retain(|(child, _)| child != name);
    children.push((name.to_owned(), id));
}

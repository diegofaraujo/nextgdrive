use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::{AccountId, DriveItemId};

/// How an account's files are on this computer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyncMode {
    /// Selected folders are copied to disk and kept in sync both ways.
    #[default]
    Mirror,
    /// The whole Drive is shown in a virtual folder; files are downloaded
    /// when opened, except those made available offline.
    Stream,
}

/// User preferences for syncing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Folder holding one subfolder per account. Always absolute.
    pub sync_root: PathBuf,
    /// Names of the folders, inside each account folder, for My Drive and for
    /// shared drives. Chosen (translated) once, so a later language change
    /// does not move the user's files.
    pub my_drive_folder: String,
    pub shared_drives_folder: String,
    /// How often background sync checks Drive for changes, in seconds.
    pub poll_interval_secs: u64,
    /// Desktop notifications about conflicts and problems.
    pub notifications: bool,
    /// Folders (or drive roots) chosen for download, per account. Everything
    /// inside a selected folder is synced. Accounts start with nothing selected.
    pub selections: BTreeMap<AccountId, BTreeSet<DriveItemId>>,
    /// Accounts in [`SyncMode::Stream`]; the rest mirror.
    pub streamed: BTreeSet<AccountId>,
    /// Streamed files and folders kept on disk in full, per account.
    pub offline: BTreeMap<AccountId, BTreeSet<DriveItemId>>,
    /// Bytes the cache of streamed files may use (offline items not counted).
    pub cache_limit: u64,
}

impl Settings {
    pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
    pub const DEFAULT_CACHE_LIMIT: u64 = 5 << 30;

    pub fn mode(&self, account: &AccountId) -> SyncMode {
        if self.streamed.contains(account) {
            SyncMode::Stream
        } else {
            SyncMode::Mirror
        }
    }

    pub fn set_mode(&mut self, account: &AccountId, mode: SyncMode) {
        match mode {
            SyncMode::Stream => self.streamed.insert(account.clone()),
            SyncMode::Mirror => self.streamed.remove(account),
        };
    }

    pub fn offline_items(&self, account: &AccountId) -> impl Iterator<Item = &DriveItemId> {
        self.offline.get(account).into_iter().flatten()
    }

    pub fn selection(&self, account: &AccountId) -> impl Iterator<Item = &DriveItemId> {
        self.selections.get(account).into_iter().flatten()
    }

    pub fn is_selected(&self, account: &AccountId, item: &DriveItemId) -> bool {
        self.selections
            .get(account)
            .is_some_and(|s| s.contains(item))
    }

    pub fn select(&mut self, account: &AccountId, item: DriveItemId) {
        self.selections
            .entry(account.clone())
            .or_default()
            .insert(item);
    }

    pub fn deselect(&mut self, account: &AccountId, item: &DriveItemId) {
        if let Some(selection) = self.selections.get_mut(account) {
            selection.remove(item);
            if selection.is_empty() {
                self.selections.remove(account);
            }
        }
    }

    /// Local folder of an account: `<sync root>/<email>`.
    pub fn account_dir(&self, account: &crate::Account) -> PathBuf {
        self.sync_root.join(account.folder_name())
    }
}

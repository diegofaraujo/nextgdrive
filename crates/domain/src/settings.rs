use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::{AccountId, DriveItemId};

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
    /// Folders (or drive roots) chosen for download, per account. Everything
    /// inside a selected folder is synced. Accounts start with nothing selected.
    pub selections: BTreeMap<AccountId, BTreeSet<DriveItemId>>,
}

impl Settings {
    pub fn selection(&self, account: &AccountId) -> impl Iterator<Item = &DriveItemId> {
        self.selections.get(account).into_iter().flatten()
    }

    pub fn is_selected(&self, account: &AccountId, item: &DriveItemId) -> bool {
        self.selections.get(account).is_some_and(|s| s.contains(item))
    }

    pub fn select(&mut self, account: &AccountId, item: DriveItemId) {
        self.selections.entry(account.clone()).or_default().insert(item);
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

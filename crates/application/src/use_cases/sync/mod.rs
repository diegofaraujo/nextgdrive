//! Two-way sync of an account's selected folders.
//!
//! Safety rules, in order of importance:
//! 1. Nothing is ever deleted permanently in Drive: local deletions move
//!    items to the Drive trash (or, for items others own, out of My Drive).
//! 2. A change is never lost: when a file changed on both sides, Drive's
//!    version keeps the name and the local one is kept (and uploaded) as a
//!    conflict copy next to it.
//! 3. The whole remote tree is listed and the local one scanned before
//!    anything changes, so a failing API call cannot look like deletions.
//! 4. If a synced folder vanished entirely (e.g. an unmounted disk), or many
//!    files were deleted at once, nothing is trashed without confirmation.
//! 5. Files are replaced only once their content is fully downloaded and its
//!    MD5 matches Drive's.

mod apply;
mod local;
mod plan;
mod remote;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use nextgdrive_domain::{Account, DriveItemId, ItemKind};

use crate::ports::{
    Authenticator, CredentialStore, DriveService, LocalFileSystem, SettingsRepository,
    SyncStateStore, SyncedItem,
};
use crate::{AppError, TokenProvider};

/// Below this many local deletions, no confirmation is ever asked.
pub const MASS_DELETION_MIN: usize = 20;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: usize,
    pub downloaded_bytes: u64,
    pub uploaded: usize,
    pub uploaded_bytes: u64,
    /// Local items moved or renamed to follow Drive.
    pub moved: usize,
    /// Drive items moved or renamed to follow local changes.
    pub moved_in_drive: usize,
    /// Local copies removed because their items were deleted in Drive.
    pub removed: usize,
    /// Drive items moved to the trash (or out of My Drive) because they were
    /// deleted locally.
    pub trashed: usize,
    /// Local versions of files that also changed in Drive, kept under a new
    /// name (and uploaded).
    pub conflict_copies: Vec<PathBuf>,
    /// Items left alone.
    pub conflicts: Vec<Conflict>,
    /// Drive items that have no local counterpart.
    pub skipped: Vec<(PathBuf, SkipReason)>,
    /// Items that failed; the rest of the sync went on.
    pub failures: Vec<(PathBuf, AppError)>,
}

impl SyncReport {
    fn conflict(&mut self, path: PathBuf, reason: ConflictReason) {
        self.conflicts.push(Conflict { path, reason });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: PathBuf,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    /// A file where Drive has a folder, or the other way around.
    TypeClash,
    /// Changed locally, but the user may not edit it in Drive.
    ReadOnly,
    /// Deleted locally, but the user may not delete it in Drive; restored.
    CannotDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Shortcut,
    NotDownloadable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncProgress {
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// What to do with files deleted on this computer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Deletions {
    /// Trash them in Drive, unless they are many: then stop with
    /// [`AppError::MassDeletion`] and let the user decide.
    #[default]
    Ask,
    /// Trash them in Drive however many they are (the user confirmed).
    Trash,
    /// Download them again.
    Restore,
}

#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// Inserted in the names of conflict copies, e.g. "conflicted copy
    /// 2026-10-23 14-30". Front-ends translate it.
    pub conflict_label: String,
    pub deletions: Deletions,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            conflict_label: "conflicted copy".into(),
            deletions: Deletions::Ask,
        }
    }
}

pub struct SyncAccount<'a, A, C, D, F, S, T> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    fs: &'a F,
    settings: &'a S,
    state: &'a T,
}

impl<'a, A, C, D, F, S, T> SyncAccount<'a, A, C, D, F, S, T>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    F: LocalFileSystem,
    S: SettingsRepository,
    T: SyncStateStore,
{
    pub fn new(
        tokens: &'a TokenProvider<A, C>,
        drive: &'a D,
        fs: &'a F,
        settings: &'a S,
        state: &'a T,
    ) -> Self {
        Self {
            tokens,
            drive,
            fs,
            settings,
            state,
        }
    }

    pub async fn execute(
        &self,
        account: &Account,
        options: &SyncOptions,
        on_progress: &(dyn Fn(&SyncProgress) + Sync),
    ) -> Result<SyncReport, AppError> {
        let _lock = self
            .state
            .try_lock(&account.id)
            .map_err(AppError::Storage)?
            .ok_or_else(|| AppError::SyncInProgress(account.email.clone()))?;
        let settings = self.settings.load().map_err(AppError::Storage)?;
        let dir = settings.account_dir(account);
        let mut report = SyncReport::default();

        let token = self.tokens.access_token(account).await?;
        let (remote, roots) =
            remote::list(self.drive, &token, account, &settings, &mut report).await?;
        let state: HashMap<DriveItemId, SyncedItem> = self
            .state
            .items(&account.id)
            .map_err(AppError::Storage)?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        let local = local::scan(self.fs, &dir, &roots)?;

        // A selected folder that was synced before and is now gone entirely is
        // far more likely an unmounted disk than a wish to delete everything.
        for root in &roots {
            let synced_before = state.values().any(|s| s.selection == root.selection);
            if synced_before && !local.contains_key(&root.path) {
                return Err(AppError::LocalFolderMissing(dir.join(&root.path)));
            }
        }

        let missing = match options.deletions {
            Deletions::Restore => plan::MissingFiles::Restore,
            Deletions::Ask | Deletions::Trash => plan::MissingFiles::Trash,
        };
        let is_selected = |selection: &DriveItemId| settings.is_selected(&account.id, selection);
        let plan = plan::plan(remote, &roots, local, &state, &is_selected, missing);
        if options.deletions == Deletions::Ask
            && plan.deletions > MASS_DELETION_MIN
            && plan.deletions * 2 > plan.tracked_files
        {
            return Err(AppError::MassDeletion {
                count: plan.deletions,
            });
        }

        let folders: BTreeMap<PathBuf, DriveItemId> = plan
            .remote
            .values()
            .filter(|w| w.item.kind == ItemKind::Folder)
            .map(|w| (w.path.clone(), w.item.id.clone()))
            .collect();
        let mut executor = apply::Executor {
            tokens: self.tokens,
            drive: self.drive,
            fs: self.fs,
            store: self.state,
            account,
            dir,
            conflict_label: options.conflict_label.clone(),
            remote: plan.remote,
            state,
            folders,
            report,
            progress: SyncProgress::default(),
        };
        executor.run(plan.ops, on_progress).await?;
        Ok(executor.report)
    }
}

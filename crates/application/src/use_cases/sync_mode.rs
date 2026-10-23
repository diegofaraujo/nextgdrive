use std::path::{Path, PathBuf};

use nextgdrive_domain::{Account, SyncMode};

use crate::AppError;
use crate::ports::{LocalFileSystem, SettingsRepository, StagingArea, SyncStateStore};
use crate::use_cases::local_copies::{Removal, remove_local_copy};

/// What switching an account's mode did to its folder.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModeChange {
    /// Mirrored copies removed: they are in Drive, and the streamed folder
    /// shows them again.
    pub removed: usize,
    /// Where files that exist only here (changed, or never synced) were
    /// moved, so the account folder can hold the streamed Drive.
    pub kept_folder: Option<PathBuf>,
}

/// Switches an account between mirroring and streaming. The folder is the
/// same, so what is in it has to be dealt with first; nothing that exists
/// only on this computer is ever deleted.
pub struct SetSyncMode<'a, S, T, F, G> {
    settings: &'a S,
    state: &'a T,
    fs: &'a F,
    /// The account's streamed files not uploaded yet.
    staging: &'a G,
}

impl<'a, S, T, F, G> SetSyncMode<'a, S, T, F, G>
where
    S: SettingsRepository,
    T: SyncStateStore,
    F: LocalFileSystem,
    G: StagingArea,
{
    pub fn new(settings: &'a S, state: &'a T, fs: &'a F, staging: &'a G) -> Self {
        Self {
            settings,
            state,
            fs,
            staging,
        }
    }

    /// `kept_label` names the folder for files kept aside, e.g. "kept files"
    /// for `ana@x.com (kept files)`.
    pub fn execute(
        &self,
        account: &Account,
        mode: SyncMode,
        kept_label: &str,
    ) -> Result<ModeChange, AppError> {
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        if settings.mode(&account.id) == mode {
            return Ok(ModeChange::default());
        }
        let _lock = self
            .state
            .try_lock(&account.id)
            .map_err(AppError::Storage)?
            .ok_or_else(|| AppError::SyncInProgress(account.email.clone()))?;
        let dir = settings.account_dir(account);
        let mut change = ModeChange::default();
        match mode {
            SyncMode::Stream => {
                change.removed = self.remove_mirrored_copies(account, &dir)?;
                prune_empty_dirs(self.fs, &dir)?;
                let leftovers = self.fs.metadata(&dir).map_err(AppError::Storage)?.is_some()
                    && !self
                        .fs
                        .list_dir(&dir)
                        .map_err(AppError::Storage)?
                        .is_empty();
                if leftovers {
                    let kept = self.free_sibling(&dir, kept_label)?;
                    self.fs.rename(&dir, &kept).map_err(AppError::Storage)?;
                    change.kept_folder = Some(kept);
                }
                settings.selections.remove(&account.id);
            }
            SyncMode::Mirror => {
                let pending = self.staging.list().map_err(AppError::Storage)?.len();
                if pending > 0 {
                    return Err(AppError::PendingUploads(pending));
                }
                settings.offline.remove(&account.id);
            }
        }
        settings.set_mode(&account.id, mode);
        self.settings.save(&settings).map_err(AppError::Storage)?;
        Ok(change)
    }

    /// Removes mirrored copies unchanged since their last sync, and forgets
    /// every synced item. Changed copies stay where they are.
    fn remove_mirrored_copies(&self, account: &Account, dir: &Path) -> Result<usize, AppError> {
        let mut items = self.state.items(&account.id).map_err(AppError::Storage)?;
        // Deepest first, so folders are empty by the time they are reached.
        items.sort_by_key(|item| std::cmp::Reverse(item.path.components().count()));
        let mut removed = 0;
        for item in items {
            if let Removal::Removed = remove_local_copy(self.fs, &dir.join(&item.path), &item)
                .map_err(AppError::Storage)?
            {
                removed += 1;
            }
            self.state
                .remove(&account.id, &item.id)
                .map_err(AppError::Storage)?;
        }
        Ok(removed)
    }

    /// `<dir> (<label>)`, or `<dir> (<label> 2)`… if taken.
    fn free_sibling(&self, dir: &Path, label: &str) -> Result<PathBuf, AppError> {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for n in 1.. {
            let suffix = if n == 1 {
                label.to_owned()
            } else {
                format!("{label} {n}")
            };
            let candidate = dir.with_file_name(format!("{name} ({suffix})"));
            if self
                .fs
                .metadata(&candidate)
                .map_err(AppError::Storage)?
                .is_none()
            {
                return Ok(candidate);
            }
        }
        unreachable!("an unused name always exists")
    }
}

/// Removes folders under `dir` that hold nothing but other empty folders.
fn prune_empty_dirs<F: LocalFileSystem>(fs: &F, dir: &Path) -> Result<(), AppError> {
    if !fs
        .metadata(dir)
        .map_err(AppError::Storage)?
        .is_some_and(|m| m.is_dir)
    {
        return Ok(());
    }
    for entry in fs.list_dir(dir).map_err(AppError::Storage)? {
        if entry.metadata.is_dir {
            let child = dir.join(&entry.name);
            prune_empty_dirs(fs, &child)?;
            fs.remove_dir_if_empty(&child).map_err(AppError::Storage)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nextgdrive_domain::DriveItemId;

    use super::*;
    use crate::TokenProvider;
    use crate::ports::{StagedFile, StagedTarget};
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, FakeFileSystem, InMemoryCredentials, InMemorySettings,
        InMemoryStaging, InMemorySyncState, account,
    };
    use crate::use_cases::{SyncAccount, SyncOptions};

    const ACCOUNT_DIR: &str = "/home/ana/GoogleDrive/ana@x.com";

    struct World {
        ana: Account,
        tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
        drive: FakeDrive,
        fs: FakeFileSystem,
        settings: InMemorySettings,
        state: InMemorySyncState,
        staging: InMemoryStaging,
    }

    /// Mirroring `Docs` (inside My Drive), already synced.
    async fn mirrored() -> World {
        let ana = account("1", "ana@x.com");
        let w = World {
            tokens: TokenProvider::new(
                FakeAuth::default(),
                InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
            ),
            drive: FakeDrive::new(ana.clone())
                .with_folder("root", "docs", "Docs")
                .with_content("docs", "a", "a.txt", "alpha")
                .with_content("docs", "b", "b.txt", "beta"),
            ana,
            fs: FakeFileSystem::default(),
            settings: InMemorySettings::default(),
            state: InMemorySyncState::default(),
            staging: InMemoryStaging::default(),
        };
        w.settings
            .update(|s| s.select(&w.ana.id, DriveItemId::new("docs")));
        SyncAccount::new(&w.tokens, &w.drive, &w.fs, &w.settings, &w.state)
            .execute(&w.ana, &SyncOptions::default(), &|_| {})
            .await
            .unwrap();
        w
    }

    impl World {
        fn switch(&self, mode: SyncMode) -> Result<ModeChange, AppError> {
            SetSyncMode::new(&self.settings, &self.state, &self.fs, &self.staging)
                .execute(&self.ana, mode, "kept")
        }
    }

    #[tokio::test]
    async fn streaming_clears_the_mirrored_copies() {
        let w = mirrored().await;

        let change = w.switch(SyncMode::Stream).unwrap();

        assert_eq!(change.kept_folder, None);
        assert!(change.removed >= 2);
        assert!(!w.fs.is_dir(ACCOUNT_DIR) || w.fs.files_under(ACCOUNT_DIR).is_empty());
        assert!(!w.fs.is_dir(&format!("{ACCOUNT_DIR}/My Drive")));
        let settings = w.settings.load().unwrap();
        assert_eq!(settings.mode(&w.ana.id), SyncMode::Stream);
        assert!(settings.selections.is_empty());
        assert!(w.state.items(&w.ana.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn files_that_exist_only_here_are_moved_aside_never_deleted() {
        let w = mirrored().await;
        w.fs.write_local(
            &format!("{ACCOUNT_DIR}/My Drive/Docs/a.txt"),
            "changed here",
        );
        w.fs.write_local(&format!("{ACCOUNT_DIR}/notes.txt"), "never synced");

        let change = w.switch(SyncMode::Stream).unwrap();

        let kept = "/home/ana/GoogleDrive/ana@x.com (kept)";
        assert_eq!(change.kept_folder.as_deref(), Some(Path::new(kept)));
        assert_eq!(w.fs.files_under(kept), ["My Drive/Docs/a.txt", "notes.txt"]);
        assert_eq!(
            w.fs.read(&format!("{kept}/My Drive/Docs/a.txt")).as_deref(),
            Some("changed here")
        );
    }

    #[tokio::test]
    async fn mirroring_again_waits_for_pending_uploads() {
        let w = mirrored().await;
        w.switch(SyncMode::Stream).unwrap();
        w.staging
            .create(&StagedFile {
                key: "k".into(),
                name: "draft.txt".into(),
                target: StagedTarget::New {
                    parent: DriveItemId::new("docs"),
                },
                modified: std::time::SystemTime::UNIX_EPOCH,
            })
            .unwrap();

        assert!(matches!(
            w.switch(SyncMode::Mirror),
            Err(AppError::PendingUploads(1))
        ));
        w.staging.remove("k").unwrap();
        w.switch(SyncMode::Mirror).unwrap();
        assert_eq!(w.settings.load().unwrap().mode(&w.ana.id), SyncMode::Mirror);
    }

    #[tokio::test]
    async fn switching_waits_for_a_running_sync() {
        let w = mirrored().await;
        let _held = w.state.try_lock(&w.ana.id).unwrap().unwrap();
        assert!(matches!(
            w.switch(SyncMode::Stream),
            Err(AppError::SyncInProgress(_))
        ));
    }
}

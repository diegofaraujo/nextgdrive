use std::path::PathBuf;

use nextgdrive_domain::{Account, DriveItemId, ItemKind};

use crate::ports::{
    Authenticator, CredentialStore, DriveService, LocalFileSystem, SettingsRepository,
    SyncStateStore,
};
use crate::use_cases::local_copies::{Removal, remove_local_copy};
use crate::{AppError, TokenProvider};

/// What to do with the files already downloaded from a folder being deselected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCopies {
    /// Leave them as regular files that are no longer updated.
    Keep,
    /// Delete the ones unchanged since the last sync; they stay in Drive.
    Remove,
}

#[derive(Debug, Default)]
pub struct DeselectReport {
    pub removed: usize,
    /// Files left in place because they changed on this computer.
    pub kept: Vec<PathBuf>,
    pub failures: Vec<(PathBuf, AppError)>,
}

/// Chooses which folders of an account are synced.
pub struct ChangeSelection<'a, A, C, D, S, F, T> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    settings: &'a S,
    fs: &'a F,
    state: &'a T,
}

impl<'a, A, C, D, S, F, T> ChangeSelection<'a, A, C, D, S, F, T>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    S: SettingsRepository,
    F: LocalFileSystem,
    T: SyncStateStore,
{
    pub fn new(
        tokens: &'a TokenProvider<A, C>,
        drive: &'a D,
        settings: &'a S,
        fs: &'a F,
        state: &'a T,
    ) -> Self {
        Self {
            tokens,
            drive,
            settings,
            fs,
            state,
        }
    }

    /// Selects a folder or a drive root (whose id is also a folder in Drive).
    pub async fn select(&self, account: &Account, folder: &DriveItemId) -> Result<(), AppError> {
        let token = self.tokens.access_token(account).await?;
        let item = self
            .drive
            .item(&token, folder)
            .await
            .map_err(AppError::Drive)?;
        if item.kind != ItemKind::Folder {
            return Err(AppError::NotAFolder(folder.clone()));
        }
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        // Drive resolves aliases such as "root" to the real id; store that.
        settings.select(&account.id, item.id);
        self.settings.save(&settings).map_err(AppError::Storage)
    }

    /// Stops syncing a folder and handles its local copies right away.
    pub fn deselect(
        &self,
        account: &Account,
        folder: &DriveItemId,
        copies: LocalCopies,
    ) -> Result<DeselectReport, AppError> {
        // Removing files while a sync writes them would race.
        let _lock = self
            .state
            .try_lock(&account.id)
            .map_err(AppError::Storage)?
            .ok_or_else(|| AppError::SyncInProgress(account.email.clone()))?;
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        settings.deselect(&account.id, folder);
        self.settings.save(&settings).map_err(AppError::Storage)?;

        let dir = settings.account_dir(account);
        let mut synced: Vec<_> = self
            .state
            .items(&account.id)
            .map_err(AppError::Storage)?
            .into_iter()
            .filter(|item| item.selection == *folder)
            .collect();
        // Deepest first, so folders are empty by the time they are reached.
        synced.sort_by_key(|item| std::cmp::Reverse(item.path.components().count()));

        let mut report = DeselectReport::default();
        for item in synced {
            if copies == LocalCopies::Remove {
                let path = dir.join(&item.path);
                match remove_local_copy(self.fs, &path, &item) {
                    Ok(Removal::Removed) => report.removed += 1,
                    Ok(Removal::KeptChanged) => report.kept.push(path),
                    Ok(Removal::KeptNotEmpty | Removal::Absent) => {}
                    Err(e) => report.failures.push((path, AppError::Storage(e))),
                }
            }
            self.state
                .remove(&account.id, &item.id)
                .map_err(AppError::Storage)?;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::SyncAccount;
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, FakeFileSystem, InMemoryCredentials, InMemorySettings,
        InMemorySyncState, account,
    };

    const DIR: &str = "/home/ana/GoogleDrive/ana@x.com/My Drive";

    struct World {
        ana: Account,
        tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
        drive: FakeDrive,
        settings: InMemorySettings,
        fs: FakeFileSystem,
        state: InMemorySyncState,
    }

    fn world() -> World {
        let ana = account("1", "ana@x.com");
        World {
            tokens: TokenProvider::new(
                FakeAuth::default(),
                InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
            ),
            drive: FakeDrive::new(ana.clone())
                .with_folder("root", "docs", "Docs")
                .with_content("docs", "a", "a.txt", "alpha")
                .with_folder("docs", "sub", "Sub")
                .with_content("sub", "b", "b.txt", "beta")
                .with_file("root", "f", "file.txt"),
            ana,
            settings: InMemorySettings::default(),
            fs: FakeFileSystem::default(),
            state: InMemorySyncState::default(),
        }
    }

    impl World {
        fn change(
            &self,
        ) -> ChangeSelection<
            '_,
            FakeAuth,
            InMemoryCredentials,
            FakeDrive,
            InMemorySettings,
            FakeFileSystem,
            InMemorySyncState,
        > {
            ChangeSelection::new(
                &self.tokens,
                &self.drive,
                &self.settings,
                &self.fs,
                &self.state,
            )
        }

        async fn select_and_sync(&self) {
            self.change()
                .select(&self.ana, &DriveItemId::new("docs"))
                .await
                .unwrap();
            SyncAccount::new(
                &self.tokens,
                &self.drive,
                &self.fs,
                &self.settings,
                &self.state,
            )
            .execute(&self.ana, &|_| {})
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn selects_folders_but_not_files() {
        let w = world();
        w.change()
            .select(&w.ana, &DriveItemId::new("docs"))
            .await
            .unwrap();
        assert!(
            w.settings
                .load()
                .unwrap()
                .is_selected(&w.ana.id, &DriveItemId::new("docs"))
        );

        let result = w.change().select(&w.ana, &DriveItemId::new("f")).await;
        assert!(matches!(result, Err(AppError::NotAFolder(_))));
    }

    #[tokio::test]
    async fn deselect_can_keep_local_copies() {
        let w = world();
        w.select_and_sync().await;

        let report = w
            .change()
            .deselect(&w.ana, &DriveItemId::new("docs"), LocalCopies::Keep)
            .unwrap();

        assert_eq!(report.removed, 0);
        assert_eq!(w.fs.files_under(DIR), ["Docs/Sub/b.txt", "Docs/a.txt"]);
        assert!(w.settings.load().unwrap().selections.is_empty());
        assert!(w.state.items(&w.ana.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn deselect_can_remove_unchanged_copies_only() {
        let w = world();
        w.select_and_sync().await;
        w.fs.write_local(&format!("{DIR}/Docs/Sub/b.txt"), "my edit");

        let report = w
            .change()
            .deselect(&w.ana, &DriveItemId::new("docs"), LocalCopies::Remove)
            .unwrap();

        assert_eq!(w.fs.files_under(DIR), ["Docs/Sub/b.txt"]);
        assert_eq!(
            report.kept,
            [PathBuf::from(format!("{DIR}/Docs/Sub/b.txt"))]
        );
        // a.txt removed; Docs and Sub still hold the edited file.
        assert_eq!(report.removed, 1);
        assert!(w.state.items(&w.ana.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_everything_removes_the_folders_too() {
        let w = world();
        w.select_and_sync().await;

        let report = w
            .change()
            .deselect(&w.ana, &DriveItemId::new("docs"), LocalCopies::Remove)
            .unwrap();

        assert!(w.fs.files_under(DIR).is_empty());
        assert!(!w.fs.is_dir(&format!("{DIR}/Docs")));
        // a.txt, b.txt, Sub and Docs.
        assert_eq!(report.removed, 4);
    }
}

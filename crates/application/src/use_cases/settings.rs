use std::path::PathBuf;

use nextgdrive_domain::Settings;

use crate::AppError;
use crate::ports::{AccountRepository, LocalFileSystem, SettingsRepository, SyncStateStore};

pub struct GetSettings<'a, S> {
    settings: &'a S,
}

impl<'a, S: SettingsRepository> GetSettings<'a, S> {
    pub fn new(settings: &'a S) -> Self {
        Self { settings }
    }

    pub fn execute(&self) -> Result<Settings, AppError> {
        self.settings.load().map_err(AppError::Storage)
    }
}

/// Turns desktop notifications on or off.
pub struct SetNotifications<'a, S> {
    settings: &'a S,
}

impl<'a, S: SettingsRepository> SetNotifications<'a, S> {
    pub fn new(settings: &'a S) -> Self {
        Self { settings }
    }

    pub fn execute(&self, enabled: bool) -> Result<(), AppError> {
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        settings.notifications = enabled;
        self.settings.save(&settings).map_err(AppError::Storage)
    }
}

/// Changes the folder that holds the synced accounts, moving each account's
/// folder there. If a move fails, the setting is left unchanged.
pub struct SetSyncRoot<'a, S, R, F, T> {
    settings: &'a S,
    accounts: &'a R,
    fs: &'a F,
    state: &'a T,
}

impl<'a, S, R, F, T> SetSyncRoot<'a, S, R, F, T>
where
    S: SettingsRepository,
    R: AccountRepository,
    F: LocalFileSystem,
    T: SyncStateStore,
{
    pub fn new(settings: &'a S, accounts: &'a R, fs: &'a F, state: &'a T) -> Self {
        Self {
            settings,
            accounts,
            fs,
            state,
        }
    }

    pub fn execute(&self, root: PathBuf) -> Result<(), AppError> {
        if !root.is_absolute() {
            return Err(AppError::InvalidSyncRoot(root));
        }
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        if settings.sync_root == root {
            return Ok(());
        }
        let accounts = self.accounts.list().map_err(AppError::Storage)?;
        // Nothing may sync while folders move.
        let mut locks = Vec::new();
        for account in &accounts {
            let lock = self
                .state
                .try_lock(&account.id)
                .map_err(AppError::Storage)?;
            locks.push(lock.ok_or_else(|| AppError::SyncInProgress(account.email.clone()))?);
        }
        // Only folders this installation synced are ours to move; a folder that
        // merely has the same name (another installation's, a backup…) is not.
        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        for account in &accounts {
            if !self
                .state
                .items(&account.id)
                .map_err(AppError::Storage)?
                .is_empty()
            {
                moves.push((
                    settings.account_dir(account),
                    root.join(account.folder_name()),
                ));
            }
        }
        for (from, to) in &moves {
            let from_exists = self.fs.metadata(from).map_err(AppError::Storage)?.is_some();
            if from_exists && self.fs.metadata(to).map_err(AppError::Storage)?.is_some() {
                return Err(AppError::FolderExists(to.clone()));
            }
        }
        self.fs.create_dir_all(&root).map_err(AppError::Storage)?;
        let mut done: Vec<&(PathBuf, PathBuf)> = Vec::new();
        for pair @ (from, to) in &moves {
            if self.fs.metadata(from).map_err(AppError::Storage)?.is_none() {
                continue;
            }
            if let Err(e) = self.fs.rename(from, to) {
                // Put back what was already moved, so nothing is split.
                for (back_from, back_to) in done.iter().rev() {
                    let _ = self.fs.rename(back_to, back_from);
                }
                return Err(AppError::Storage(e));
            }
            done.push(pair);
        }
        settings.sync_root = root;
        self.settings.save(&settings).map_err(AppError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{
        FakeFileSystem, InMemoryAccounts, InMemorySettings, InMemorySyncState, account,
    };

    struct World {
        settings: InMemorySettings,
        accounts: InMemoryAccounts,
        fs: FakeFileSystem,
        state: InMemorySyncState,
    }

    fn world() -> World {
        World {
            settings: InMemorySettings::default(),
            accounts: InMemoryAccounts::with([account("1", "ana@x.com")]),
            fs: FakeFileSystem::default(),
            state: InMemorySyncState::default(),
        }
    }

    impl World {
        fn set(&self, root: &str) -> Result<(), AppError> {
            SetSyncRoot::new(&self.settings, &self.accounts, &self.fs, &self.state)
                .execute(root.into())
        }
    }

    #[test]
    fn sync_root_must_be_absolute() {
        let w = world();
        assert!(matches!(
            w.set("relative/dir"),
            Err(AppError::InvalidSyncRoot(_))
        ));
    }

    /// Marks the account as synced by this installation.
    fn synced(w: &World) {
        let item = crate::ports::SyncedItem {
            id: nextgdrive_domain::DriveItemId::new("a"),
            selection: nextgdrive_domain::DriveItemId::new("root"),
            path: "My Drive/a.txt".into(),
            kind: crate::ports::SyncedKind::File,
            remote_modified: std::time::SystemTime::UNIX_EPOCH,
            remote_md5: None,
            local_size: 5,
            local_modified: std::time::SystemTime::UNIX_EPOCH,
            local_inode: 1,
        };
        w.state
            .upsert(&account("1", "ana@x.com").id, &item)
            .unwrap();
    }

    #[test]
    fn moves_account_folders_to_the_new_root() {
        let w = world();
        synced(&w);
        w.fs.write_local("/home/ana/GoogleDrive/ana@x.com/My Drive/a.txt", "alpha");

        w.set("/data/drive").unwrap();

        assert_eq!(
            w.settings.load().unwrap().sync_root,
            PathBuf::from("/data/drive")
        );
        assert_eq!(
            w.fs.read("/data/drive/ana@x.com/My Drive/a.txt").as_deref(),
            Some("alpha")
        );
        assert!(!w.fs.is_dir("/home/ana/GoogleDrive/ana@x.com"));
    }

    #[test]
    fn leaves_folders_it_never_synced_alone() {
        let w = world();
        w.fs.write_local(
            "/home/ana/GoogleDrive/ana@x.com/My Drive/a.txt",
            "someone else's",
        );

        w.set("/data/drive").unwrap();

        assert_eq!(
            w.settings.load().unwrap().sync_root,
            PathBuf::from("/data/drive")
        );
        assert!(
            w.fs.read("/home/ana/GoogleDrive/ana@x.com/My Drive/a.txt")
                .is_some()
        );
        assert!(!w.fs.is_dir("/data/drive/ana@x.com"));
    }

    #[test]
    fn refuses_to_merge_into_an_existing_folder() {
        let w = world();
        synced(&w);
        w.fs.write_local("/home/ana/GoogleDrive/ana@x.com/a.txt", "alpha");
        w.fs.write_local("/data/drive/ana@x.com/other.txt", "other");

        assert!(matches!(
            w.set("/data/drive"),
            Err(AppError::FolderExists(_))
        ));
        assert_eq!(
            w.settings.load().unwrap().sync_root,
            PathBuf::from("/home/ana/GoogleDrive")
        );
    }

    #[test]
    fn waits_for_running_syncs() {
        let w = world();
        let _held = w.state.try_lock(&account("1", "ana@x.com").id).unwrap();
        assert!(matches!(
            w.set("/data/drive"),
            Err(AppError::SyncInProgress(_))
        ));
    }
}

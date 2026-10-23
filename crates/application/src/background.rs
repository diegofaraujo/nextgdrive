//! Keeps accounts in sync over time. Front-ends call [`BackgroundSync::tick`]
//! periodically (the GUI tray app, `nextgdrive daemon`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nextgdrive_domain::{Account, AccountId, DriveItemId};

use crate::ports::{
    AccountRepository, Authenticator, CredentialStore, DriveService, LocalFileSystem,
    SettingsRepository, SyncStateStore,
};
use crate::use_cases::{Deletions, SyncAccount, SyncOptions, SyncProgress, SyncReport};
use crate::{AppError, TokenProvider};

/// Even without reported changes, sync this often: a safety net for local
/// changes the file watcher missed (it cannot see changes made while the
/// app was not running, for example).
pub const FULL_SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
pub enum SyncEvent<'a> {
    Started(&'a Account),
    Progress(&'a Account, &'a SyncProgress),
    Finished(&'a Account, &'a Result<SyncReport, AppError>),
    /// Checked; nothing to do.
    UpToDate(&'a Account),
    /// Could not even check (network, credentials…).
    CheckFailed(&'a Account, &'a AppError),
}

#[derive(Debug, Clone, Default)]
pub struct TickOptions {
    /// Sync every account with a selection, changed or not.
    pub force: bool,
    /// See [`SyncOptions::conflict_label`].
    pub conflict_label: String,
    /// Accounts whose many local deletions the user confirmed.
    pub confirmed_deletions: HashSet<AccountId>,
    /// Something changed in the local folders since the last tick (the
    /// change feed only reports changes in Drive).
    pub local_changes: bool,
}

/// What was true at an account's last successful sync.
struct Memory {
    selection: BTreeSet<DriveItemId>,
    synced_at: Instant,
}

pub struct BackgroundSync<'a, A, C, D, F, S, T, R> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    fs: &'a F,
    settings: &'a S,
    state: &'a T,
    accounts: &'a R,
    memory: Mutex<HashMap<AccountId, Memory>>,
}

impl<'a, A, C, D, F, S, T, R> BackgroundSync<'a, A, C, D, F, S, T, R>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    F: LocalFileSystem,
    S: SettingsRepository,
    T: SyncStateStore,
    R: AccountRepository,
{
    pub fn new(
        tokens: &'a TokenProvider<A, C>,
        drive: &'a D,
        fs: &'a F,
        settings: &'a S,
        state: &'a T,
        accounts: &'a R,
    ) -> Self {
        Self {
            tokens,
            drive,
            fs,
            settings,
            state,
            accounts,
            memory: Mutex::default(),
        }
    }

    /// Checks every account and syncs those that need it. `force` syncs all
    /// accounts with a selection, changed or not. Only fails if the accounts
    /// or settings cannot be read; per-account problems become events.
    pub async fn tick(
        &self,
        options: &TickOptions,
        on_event: &(dyn Fn(SyncEvent<'_>) + Sync),
    ) -> Result<(), AppError> {
        let settings = self.settings.load().map_err(AppError::Storage)?;
        for account in self.accounts.list().map_err(AppError::Storage)? {
            // Streamed accounts are kept up to date by their mount.
            if settings.mode(&account.id) == nextgdrive_domain::SyncMode::Stream {
                continue;
            }
            let selection: BTreeSet<_> = settings.selection(&account.id).cloned().collect();
            if selection.is_empty() {
                continue;
            }
            self.check(&account, selection, options, on_event).await;
        }
        Ok(())
    }

    async fn check(
        &self,
        account: &Account,
        selection: BTreeSet<DriveItemId>,
        options: &TickOptions,
        on_event: &(dyn Fn(SyncEvent<'_>) + Sync),
    ) {
        let changes = async {
            let cursor = self.state.cursor(&account.id).map_err(AppError::Storage)?;
            let token = self.tokens.access_token(account).await?;
            self.drive
                .changes_since(&token, cursor.as_deref())
                .await
                .map_err(AppError::Drive)
        }
        .await;
        let changes = match changes {
            Ok(changes) => changes,
            Err(e) => return on_event(SyncEvent::CheckFailed(account, &e)),
        };

        let stale = self.lock_memory().get(&account.id).is_none_or(|m| {
            m.selection != selection || m.synced_at.elapsed() >= FULL_SYNC_INTERVAL
        });
        let confirmed = options.confirmed_deletions.contains(&account.id);
        if !(options.force || options.local_changes || confirmed || changes.any || stale) {
            return on_event(SyncEvent::UpToDate(account));
        }

        on_event(SyncEvent::Started(account));
        let on_progress = |p: &SyncProgress| on_event(SyncEvent::Progress(account, p));
        let result = SyncAccount::new(self.tokens, self.drive, self.fs, self.settings, self.state)
            .execute(
                account,
                &SyncOptions {
                    conflict_label: options.conflict_label.clone(),
                    deletions: if confirmed {
                        Deletions::Trash
                    } else {
                        Deletions::Ask
                    },
                },
                &on_progress,
            )
            .await;
        // Advance in the change feed only once everything up to it is synced;
        // otherwise the next tick tries again.
        let result = result.and_then(|report| {
            self.state
                .set_cursor(&account.id, &changes.cursor)
                .map_err(AppError::Storage)?;
            self.lock_memory().insert(
                account.id.clone(),
                Memory {
                    selection,
                    synced_at: Instant::now(),
                },
            );
            Ok(report)
        });
        on_event(SyncEvent::Finished(account, &result));
    }

    fn lock_memory(&self) -> std::sync::MutexGuard<'_, HashMap<AccountId, Memory>> {
        self.memory.lock().expect("lock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, FakeFileSystem, InMemoryAccounts, InMemoryCredentials,
        InMemorySettings, InMemorySyncState, account,
    };

    struct World {
        ana: Account,
        tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
        drive: FakeDrive,
        fs: FakeFileSystem,
        settings: InMemorySettings,
        state: InMemorySyncState,
        accounts: InMemoryAccounts,
    }

    fn world() -> World {
        let ana = account("1", "ana@x.com");
        let settings = InMemorySettings::default();
        settings.update(|s| s.select(&ana.id, DriveItemId::new("docs")));
        World {
            tokens: TokenProvider::new(
                FakeAuth::default(),
                InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
            ),
            drive: FakeDrive::new(ana.clone())
                .with_folder("root", "docs", "Docs")
                .with_content("docs", "a", "a.txt", "alpha"),
            fs: FakeFileSystem::default(),
            settings,
            state: InMemorySyncState::default(),
            accounts: InMemoryAccounts::with([ana.clone()]),
            ana,
        }
    }

    /// Runs a tick and returns a summary of its events.
    async fn tick(
        bg: &BackgroundSync<
            '_,
            FakeAuth,
            InMemoryCredentials,
            FakeDrive,
            FakeFileSystem,
            InMemorySettings,
            InMemorySyncState,
            InMemoryAccounts,
        >,
        force: bool,
    ) -> Vec<String> {
        let events = Mutex::new(Vec::new());
        bg.tick(
            &TickOptions {
                force,
                ..TickOptions::default()
            },
            &|event| {
                let summary = match event {
                    SyncEvent::Started(_) => "started".to_owned(),
                    SyncEvent::Progress(..) => return,
                    SyncEvent::Finished(_, Ok(r)) => format!("synced {}", r.downloaded),
                    SyncEvent::Finished(_, Err(e)) => format!("failed {e}"),
                    SyncEvent::UpToDate(_) => "up to date".to_owned(),
                    SyncEvent::CheckFailed(_, e) => format!("check failed {e}"),
                };
                events.lock().unwrap().push(summary);
            },
        )
        .await
        .unwrap();
        events.into_inner().unwrap()
    }

    fn background(
        w: &World,
    ) -> BackgroundSync<
        '_,
        FakeAuth,
        InMemoryCredentials,
        FakeDrive,
        FakeFileSystem,
        InMemorySettings,
        InMemorySyncState,
        InMemoryAccounts,
    > {
        BackgroundSync::new(
            &w.tokens,
            &w.drive,
            &w.fs,
            &w.settings,
            &w.state,
            &w.accounts,
        )
    }

    #[tokio::test]
    async fn syncs_first_then_only_when_something_changed() {
        let w = world();
        let bg = background(&w);

        assert_eq!(tick(&bg, false).await, ["started", "synced 1"]);
        assert_eq!(tick(&bg, false).await, ["up to date"]);
        assert_eq!(tick(&bg, true).await, ["started", "synced 0"]);
    }

    #[tokio::test]
    async fn remote_changes_trigger_a_sync() {
        let w = world();
        background(&w)
            .tick(&TickOptions::default(), &|_| {})
            .await
            .unwrap();
        let cursor = w.state.cursor(&w.ana.id).unwrap();
        w.drive.set_content("a", "alpha v2");

        // A new BackgroundSync has no memory: it trusts only the stored cursor.
        let bg = background(&w);
        bg.lock_memory().insert(
            w.ana.id.clone(),
            Memory {
                selection: [DriveItemId::new("docs")].into(),
                synced_at: Instant::now(),
            },
        );
        assert_eq!(tick(&bg, false).await, ["started", "synced 1"]);
        assert_ne!(w.state.cursor(&w.ana.id).unwrap(), cursor);
    }

    #[tokio::test]
    async fn selection_changes_trigger_a_sync() {
        let w = world();
        let bg = background(&w);
        tick(&bg, false).await;
        w.settings
            .update(|s| s.select(&w.ana.id, DriveItemId::new("root")));

        assert_eq!(tick(&bg, false).await, ["started", "synced 0"]);
    }

    #[tokio::test]
    async fn failed_sync_keeps_the_cursor_so_it_retries() {
        let w = world();
        w.drive.fail_listing("docs");
        let bg = background(&w);

        let events = tick(&bg, false).await;

        assert!(events[1].starts_with("failed"), "{events:?}");
        assert_eq!(w.state.cursor(&w.ana.id).unwrap(), None);
    }

    #[tokio::test]
    async fn local_changes_trigger_a_sync() {
        let w = world();
        let bg = background(&w);
        tick(&bg, false).await;
        let local = TickOptions {
            local_changes: true,
            ..TickOptions::default()
        };

        let events = Mutex::new(0);
        bg.tick(&local, &|e| {
            if matches!(e, SyncEvent::Started(_)) {
                *events.lock().unwrap() += 1;
            }
        })
        .await
        .unwrap();

        assert_eq!(*events.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn skips_streamed_accounts() {
        let w = world();
        w.settings
            .update(|s| s.set_mode(&w.ana.id, nextgdrive_domain::SyncMode::Stream));
        let bg = background(&w);

        assert!(tick(&bg, true).await.is_empty());
    }

    #[tokio::test]
    async fn skips_accounts_with_nothing_selected() {
        let w = world();
        w.settings
            .update(|s| s.deselect(&w.ana.id, &DriveItemId::new("docs")));
        let bg = background(&w);

        assert!(tick(&bg, true).await.is_empty());
        assert_eq!(
            w.drive
                .changes_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn an_account_is_never_synced_twice_at_once() {
        let w = world();
        let _held = w.state.try_lock(&w.ana.id).unwrap().unwrap();
        let bg = background(&w);

        let events = tick(&bg, false).await;

        assert_eq!(events[1], "failed ana@x.com is already being synced");
    }
}

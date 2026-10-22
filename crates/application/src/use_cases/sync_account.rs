//! Download-only sync of an account's selected folders.
//!
//! Safety rules, in order of importance:
//! 1. A local file that changed since it was last synced is never
//!    overwritten or deleted: it is reported as a conflict.
//! 2. The whole remote tree is listed before anything changes on disk, so a
//!    failing API call cannot be mistaken for "everything was deleted".
//! 3. Files are only replaced once their content is fully downloaded and its
//!    MD5 matches what Drive reports.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use nextgdrive_domain::local_name::{assign_names, google_doc_shortcut, local_name};
use nextgdrive_domain::{Account, Drive, DriveItem, DriveItemId, DriveKind, ItemKind, Settings};

use crate::ports::{
    AccessToken, Authenticator, CredentialStore, DriveService, FileWriter, LocalFileSystem,
    LocalMetadata, PortError, SettingsRepository, SyncStateStore, SyncedItem, SyncedKind,
};
use crate::use_cases::local_copies::{Removal, remove_local_copy, unchanged};
use crate::{AppError, TokenProvider};

/// Guards against malformed parent chains in Drive.
const MAX_DEPTH: usize = 256;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: usize,
    pub downloaded_bytes: u64,
    pub moved: usize,
    pub removed: usize,
    /// Local files left untouched because they differ from Drive.
    pub conflicts: Vec<Conflict>,
    /// Drive items that have no local counterpart.
    pub skipped: Vec<(PathBuf, SkipReason)>,
    /// Items that failed; the rest of the sync went on.
    pub failures: Vec<(PathBuf, AppError)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: PathBuf,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    /// Synced before, then changed on this computer.
    ChangedLocally,
    /// Something not created by Next GDrive is already at that path.
    AlreadyExists,
}

impl SyncReport {
    fn conflict(&mut self, path: PathBuf, reason: ConflictReason) {
        self.conflicts.push(Conflict { path, reason });
    }
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

pub struct SyncAccount<'a, A, C, D, F, S, T> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    fs: &'a F,
    settings: &'a S,
    state: &'a T,
}

/// An item that should exist locally after the sync.
struct Wanted {
    item: DriveItem,
    /// Relative to the account folder.
    path: PathBuf,
    selection: DriveItemId,
}

enum FileAction {
    Download,
    /// The local file already has the right content; just record it.
    Adopt(LocalMetadata),
    /// Only the recorded path changed (moved together with its folder).
    Record,
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
        on_progress: &(dyn Fn(&SyncProgress) + Sync),
    ) -> Result<SyncReport, AppError> {
        let settings = self.settings.load().map_err(AppError::Storage)?;
        let dir = settings.account_dir(account);
        let mut report = SyncReport::default();

        let wanted = self.list_remote(account, &settings, &mut report).await?;
        let mut state: HashMap<DriveItemId, SyncedItem> = self
            .state
            .items(&account.id)
            .map_err(AppError::Storage)?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();

        self.remove_gone(account, &settings, &dir, &wanted, &mut state, &mut report)?;
        self.sync_folders(account, &dir, &wanted, &mut state, &mut report)?;
        let plan = self.plan_files(&dir, &wanted, &state, &mut report).await?;
        self.apply_files(account, &dir, plan, &mut state, &mut report, on_progress)
            .await?;
        Ok(report)
    }

    /// Lists every selected folder recursively. Any API error aborts the sync.
    async fn list_remote(
        &self,
        account: &Account,
        settings: &Settings,
        report: &mut SyncReport,
    ) -> Result<BTreeMap<DriveItemId, Wanted>, AppError> {
        let token = self.tokens.access_token(account).await?;
        let drives = self.drive.drives(&token).await.map_err(AppError::Drive)?;
        let mut wanted = BTreeMap::new();
        for selection in settings.selection(&account.id) {
            if wanted.contains_key(selection) {
                continue; // Inside another selected folder.
            }
            let (root, path) = self.locate(&token, settings, &drives, selection).await?;
            wanted.insert(
                root.id.clone(),
                Wanted {
                    item: root,
                    path: path.clone(),
                    selection: selection.clone(),
                },
            );
            let mut queue = VecDeque::from([(selection.clone(), path)]);
            while let Some((folder, path)) = queue.pop_front() {
                let children = self
                    .drive
                    .children(&token, &folder)
                    .await
                    .map_err(AppError::Drive)?;
                let (local, other): (Vec<_>, Vec<_>) =
                    children.into_iter().partition(has_local_copy);
                for item in other {
                    let reason = match item.kind {
                        ItemKind::Shortcut => SkipReason::Shortcut,
                        _ => SkipReason::NotDownloadable,
                    };
                    report.skipped.push((path.join(&item.name), reason));
                }
                let names =
                    assign_names(local.iter().map(|i| (&i.id, local_name(&i.name, &i.kind))));
                for item in local {
                    if wanted.contains_key(&item.id) {
                        continue; // Seen through another parent or selection.
                    }
                    let child = path.join(&names[&item.id]);
                    if item.kind == ItemKind::Folder {
                        queue.push_back((item.id.clone(), child.clone()));
                    }
                    wanted.insert(
                        item.id.clone(),
                        Wanted {
                            item,
                            path: child,
                            selection: selection.clone(),
                        },
                    );
                }
            }
        }
        Ok(wanted)
    }

    /// Finds a selected folder and its path relative to the account folder:
    /// the drive's folder, then each ancestor's local name.
    async fn locate(
        &self,
        token: &AccessToken,
        settings: &Settings,
        drives: &[Drive],
        id: &DriveItemId,
    ) -> Result<(DriveItem, PathBuf), AppError> {
        let target = self.drive.item(token, id).await.map_err(AppError::Drive)?;
        if target.kind != ItemKind::Folder {
            return Err(AppError::NotAFolder(id.clone()));
        }
        // Climb to the drive root, remembering the chain.
        let mut chain = vec![target.clone()];
        let drive = loop {
            let top = chain.last().expect("chain is never empty");
            if let Some(drive) = drives.iter().find(|d| d.root == top.id) {
                break drive;
            }
            let parent = match top.parents.first() {
                Some(parent) if chain.len() < MAX_DEPTH => parent.clone(),
                // Shared with the user but outside any of their drives.
                _ => {
                    return Err(AppError::Drive(PortError::new(format!(
                        "{id} is not inside a drive"
                    ))));
                }
            };
            chain.push(
                self.drive
                    .item(token, &parent)
                    .await
                    .map_err(AppError::Drive)?,
            );
        };
        let mut path = drive_folder(settings, drives, drive);
        // Walk back down; each name must be unique among its siblings.
        for pair in chain.windows(2).rev() {
            let (child, parent) = (&pair[0], &pair[1]);
            let siblings = self
                .drive
                .children(token, &parent.id)
                .await
                .map_err(AppError::Drive)?;
            let names = assign_names(
                siblings
                    .iter()
                    .filter(|s| has_local_copy(s))
                    .map(|s| (&s.id, local_name(&s.name, &s.kind))),
            );
            let name = names
                .get(&child.id)
                .cloned()
                .unwrap_or_else(|| local_name(&child.name, &child.kind));
            path.push(name);
        }
        Ok((target, path))
    }

    /// Handles items recorded locally that are no longer wanted.
    fn remove_gone(
        &self,
        account: &Account,
        settings: &Settings,
        dir: &Path,
        wanted: &BTreeMap<DriveItemId, Wanted>,
        state: &mut HashMap<DriveItemId, SyncedItem>,
        report: &mut SyncReport,
    ) -> Result<(), AppError> {
        let mut gone: Vec<SyncedItem> = state
            .values()
            .filter(|s| !wanted.contains_key(&s.id))
            .cloned()
            .collect();
        // Deepest first, so folders are empty by the time they are reached.
        gone.sort_by_key(|s| std::cmp::Reverse(s.path.components().count()));
        for synced in gone {
            state.remove(&synced.id);
            self.state
                .remove(&account.id, &synced.id)
                .map_err(AppError::Storage)?;
            // Deselected: what to do with local copies was decided then
            // (see `ChangeSelection::deselect`); just stop tracking.
            if !settings.is_selected(&account.id, &synced.selection) {
                continue;
            }
            let path = dir.join(&synced.path);
            match remove_local_copy(self.fs, &path, &synced) {
                Ok(Removal::Removed) => report.removed += 1,
                Ok(Removal::KeptChanged) => report.conflict(path, ConflictReason::ChangedLocally),
                Ok(Removal::KeptNotEmpty | Removal::Absent) => {}
                Err(e) => report.failures.push((path, AppError::Storage(e))),
            }
        }
        Ok(())
    }

    /// Creates wanted folders, renaming local ones that moved on Drive.
    fn sync_folders(
        &self,
        account: &Account,
        dir: &Path,
        wanted: &BTreeMap<DriveItemId, Wanted>,
        state: &mut HashMap<DriveItemId, SyncedItem>,
        report: &mut SyncReport,
    ) -> Result<(), AppError> {
        let mut folders: Vec<&Wanted> = wanted
            .values()
            .filter(|w| w.item.kind == ItemKind::Folder)
            .collect();
        // Parents first: renaming a parent moves its children along.
        folders.sort_by_key(|w| w.path.components().count());
        for want in folders {
            let path = dir.join(&want.path);
            let old = state.get(&want.item.id).map(|s| s.path.clone());
            if let Some(old) = old.filter(|old| *old != want.path) {
                let old_path = dir.join(&old);
                if self
                    .fs
                    .metadata(&old_path)
                    .map_err(AppError::Storage)?
                    .is_some()
                    && self
                        .fs
                        .metadata(&path)
                        .map_err(AppError::Storage)?
                        .is_none()
                {
                    if let Some(parent) = path.parent() {
                        self.fs.create_dir_all(parent).map_err(AppError::Storage)?;
                    }
                    match self.fs.rename(&old_path, &path) {
                        Ok(()) => {
                            report.moved += 1;
                            self.move_descendants(account, &old, &want.path, state)?;
                        }
                        Err(e) => report.failures.push((old_path, AppError::Storage(e))),
                    }
                }
            }
            if let Err(e) = self.fs.create_dir_all(&path) {
                report.failures.push((path, AppError::Storage(e)));
                continue;
            }
            let synced = SyncedItem {
                id: want.item.id.clone(),
                selection: want.selection.clone(),
                path: want.path.clone(),
                kind: SyncedKind::Folder,
                remote_modified: want.item.modified,
                remote_md5: None,
                local_size: 0,
                local_modified: want.item.modified,
            };
            if state.get(&synced.id) != Some(&synced) {
                self.state
                    .upsert(&account.id, &synced)
                    .map_err(AppError::Storage)?;
                state.insert(synced.id.clone(), synced);
            }
        }
        Ok(())
    }

    /// After a folder moved from `old` to `new`, records the new paths of everything inside it.
    fn move_descendants(
        &self,
        account: &Account,
        old: &Path,
        new: &Path,
        state: &mut HashMap<DriveItemId, SyncedItem>,
    ) -> Result<(), AppError> {
        for synced in state.values_mut() {
            if let Ok(rest) = synced.path.strip_prefix(old)
                && !rest.as_os_str().is_empty()
            {
                synced.path = new.join(rest);
                self.state
                    .upsert(&account.id, synced)
                    .map_err(AppError::Storage)?;
            }
        }
        Ok(())
    }

    /// Decides what to do with each wanted file, without changing anything.
    /// Moves of unchanged files are applied here, since they are cheap and
    /// later decisions depend on them.
    async fn plan_files<'w>(
        &self,
        dir: &Path,
        wanted: &'w BTreeMap<DriveItemId, Wanted>,
        state: &HashMap<DriveItemId, SyncedItem>,
        report: &mut SyncReport,
    ) -> Result<Vec<(&'w Wanted, FileAction)>, AppError> {
        let mut plan = Vec::new();
        for want in wanted.values().filter(|w| w.item.kind != ItemKind::Folder) {
            let path = dir.join(&want.path);
            let mut local = self.fs.metadata(&path).map_err(AppError::Storage)?;
            let synced = state.get(&want.item.id);

            // Moved or renamed on Drive: move the local copy if it is unchanged.
            if let Some(prev) = synced.filter(|s| s.path != want.path) {
                let old_path = dir.join(&prev.path);
                match self.fs.metadata(&old_path).map_err(AppError::Storage)? {
                    // Shortcuts embed the name and may change format (e.g. the
                    // extension), so they are rewritten rather than moved.
                    Some(meta) if unchanged(prev, &meta) && prev.kind == SyncedKind::GoogleDoc => {
                        if let Err(e) = self.fs.remove_file(&old_path) {
                            report.failures.push((old_path, AppError::Storage(e)));
                            continue;
                        }
                    }
                    Some(meta) if unchanged(prev, &meta) && local.is_none() => {
                        if let Some(parent) = path.parent() {
                            self.fs.create_dir_all(parent).map_err(AppError::Storage)?;
                        }
                        match self.fs.rename(&old_path, &path) {
                            Ok(()) => {
                                report.moved += 1;
                                local = Some(meta);
                            }
                            Err(e) => {
                                report.failures.push((old_path, AppError::Storage(e)));
                                continue;
                            }
                        }
                    }
                    Some(_) => report.conflict(old_path, ConflictReason::ChangedLocally),
                    None => {}
                }
            }

            let action = match (local, synced) {
                (None, _) => FileAction::Download,
                (Some(meta), _) if meta.is_dir => {
                    report.conflict(path, ConflictReason::AlreadyExists);
                    continue;
                }
                (Some(meta), Some(prev)) if unchanged(prev, &meta) => {
                    if remote_changed(prev, &want.item) {
                        FileAction::Download
                    } else if prev.path != want.path || prev.selection != want.selection {
                        FileAction::Record
                    } else {
                        continue; // Up to date.
                    }
                }
                // Not created by us, or changed locally: only take it over if identical.
                (Some(meta), synced) => {
                    let identical = match &want.item.kind {
                        ItemKind::File { md5: Some(md5), .. } => {
                            self.fs.md5_hex(&path).await.map_err(AppError::Storage)? == *md5
                        }
                        // Shortcuts are generated, so the expected content is known exactly.
                        ItemKind::GoogleDoc { url } => {
                            let shortcut = google_doc_shortcut(&want.item.name, url);
                            self.fs
                                .has_content(&path, shortcut.as_bytes())
                                .await
                                .map_err(AppError::Storage)?
                        }
                        _ => false,
                    };
                    if identical {
                        FileAction::Adopt(meta)
                    } else {
                        let reason = match synced {
                            Some(_) => ConflictReason::ChangedLocally,
                            None => ConflictReason::AlreadyExists,
                        };
                        report.conflict(path, reason);
                        continue;
                    }
                }
            };
            plan.push((want, action));
        }
        Ok(plan)
    }

    async fn apply_files(
        &self,
        account: &Account,
        dir: &Path,
        plan: Vec<(&Wanted, FileAction)>,
        state: &mut HashMap<DriveItemId, SyncedItem>,
        report: &mut SyncReport,
        on_progress: &(dyn Fn(&SyncProgress) + Sync),
    ) -> Result<(), AppError> {
        let downloads = plan
            .iter()
            .filter(|(_, a)| matches!(a, FileAction::Download));
        let mut progress = SyncProgress {
            files_total: downloads.clone().count(),
            bytes_total: downloads.map(|(w, _)| file_size(&w.item)).sum(),
            ..SyncProgress::default()
        };
        on_progress(&progress);

        for (want, action) in plan {
            let path = dir.join(&want.path);
            let local = match action {
                FileAction::Adopt(meta) => meta,
                FileAction::Record => match state.get(&want.item.id) {
                    Some(prev) => LocalMetadata {
                        is_dir: false,
                        size: prev.local_size,
                        modified: prev.local_modified,
                    },
                    None => continue,
                },
                FileAction::Download => {
                    let result = self.download(account, want, &path).await;
                    progress.files_done += 1;
                    progress.bytes_done += file_size(&want.item);
                    on_progress(&progress);
                    match result {
                        Ok(meta) => {
                            report.downloaded += 1;
                            report.downloaded_bytes += meta.size;
                            meta
                        }
                        Err(e) => {
                            report.failures.push((path, e));
                            continue;
                        }
                    }
                }
            };
            let synced = SyncedItem {
                id: want.item.id.clone(),
                selection: want.selection.clone(),
                path: want.path.clone(),
                kind: match want.item.kind {
                    ItemKind::GoogleDoc { .. } => SyncedKind::GoogleDoc,
                    _ => SyncedKind::File,
                },
                remote_modified: want.item.modified,
                remote_md5: match &want.item.kind {
                    ItemKind::File { md5, .. } => md5.clone(),
                    _ => None,
                },
                local_size: local.size,
                local_modified: local.modified,
            };
            self.state
                .upsert(&account.id, &synced)
                .map_err(AppError::Storage)?;
            state.insert(synced.id.clone(), synced);
        }
        Ok(())
    }

    /// Writes a file's content (or a Google Doc's shortcut) into place.
    async fn download(
        &self,
        account: &Account,
        want: &Wanted,
        path: &Path,
    ) -> Result<LocalMetadata, AppError> {
        if let Some(parent) = path.parent() {
            self.fs.create_dir_all(parent).map_err(AppError::Storage)?;
        }
        let mut writer = self.fs.begin_write(path).await.map_err(AppError::Storage)?;
        match &want.item.kind {
            ItemKind::GoogleDoc { url } => {
                let shortcut = google_doc_shortcut(&want.item.name, url);
                writer
                    .write(shortcut.as_bytes())
                    .await
                    .map_err(AppError::Storage)?;
            }
            ItemKind::File { md5, .. } => {
                // Fetched per file: long syncs outlive a single access token.
                let token = self.tokens.access_token(account).await?;
                self.drive
                    .download(&token, &want.item.id, &mut writer)
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
            _ => unreachable!("only files and Google Docs are planned"),
        }
        self.fs
            .commit(writer, want.item.modified)
            .await
            .map_err(AppError::Storage)
    }
}

fn has_local_copy(item: &DriveItem) -> bool {
    matches!(
        item.kind,
        ItemKind::Folder | ItemKind::File { .. } | ItemKind::GoogleDoc { .. }
    )
}

/// Local folder of a drive, relative to the account folder.
fn drive_folder(settings: &Settings, drives: &[Drive], drive: &Drive) -> PathBuf {
    match &drive.kind {
        DriveKind::MyDrive => PathBuf::from(&settings.my_drive_folder),
        DriveKind::Shared { .. } => {
            let shared = drives.iter().filter_map(|d| match &d.kind {
                DriveKind::Shared { name } => Some((&d.root, local_name(name, &ItemKind::Folder))),
                DriveKind::MyDrive => None,
            });
            let names = assign_names(shared);
            PathBuf::from(&settings.shared_drives_folder).join(&names[&drive.root])
        }
    }
}

fn remote_changed(synced: &SyncedItem, item: &DriveItem) -> bool {
    match &item.kind {
        ItemKind::File { md5: Some(md5), .. } => synced.remote_md5.as_ref() != Some(md5),
        _ => synced.remote_modified != item.modified,
    }
}

fn file_size(item: &DriveItem) -> u64 {
    match item.kind {
        ItemKind::File { size, .. } => size,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::use_cases::fakes::{
        FakeAuth, FakeDrive, FakeFileSystem, InMemoryCredentials, InMemorySettings,
        InMemorySyncState, account,
    };

    const DIR: &str = "/home/ana/GoogleDrive/ana@x.com/My Drive";

    struct World {
        account: Account,
        tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
        drive: FakeDrive,
        fs: FakeFileSystem,
        settings: InMemorySettings,
        state: InMemorySyncState,
    }

    /// My Drive:
    ///   Docs/            (selected)
    ///     a.txt  "alpha"
    ///     Sub/b.txt "beta"
    ///     Budget         (Google Sheet)
    ///     link           (shortcut)
    ///   Other/c.txt      (not selected)
    fn world() -> World {
        let account = account("1", "ana@x.com");
        let drive = FakeDrive::new(account.clone())
            .with_folder("root", "docs", "Docs")
            .with_content("docs", "a", "a.txt", "alpha")
            .with_folder("docs", "sub", "Sub")
            .with_content("sub", "b", "b.txt", "beta")
            .with_item(
                "docs",
                "sheet",
                "Budget",
                ItemKind::GoogleDoc {
                    url: "https://docs.google.com/spreadsheets/d/sheet".into(),
                },
            )
            .with_item("docs", "link", "link", ItemKind::Shortcut)
            .with_folder("root", "other", "Other")
            .with_content("other", "c", "c.txt", "gamma");
        let settings = InMemorySettings::default();
        settings.update(|s| s.select(&account.id, DriveItemId::new("docs")));
        World {
            tokens: TokenProvider::new(
                FakeAuth::default(),
                InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
            ),
            account,
            drive,
            fs: FakeFileSystem::default(),
            settings,
            state: InMemorySyncState::default(),
        }
    }

    fn conflict(path: &str, reason: ConflictReason) -> Conflict {
        Conflict {
            path: PathBuf::from(format!("{DIR}/{path}")),
            reason,
        }
    }

    impl World {
        async fn sync(&self) -> SyncReport {
            SyncAccount::new(
                &self.tokens,
                &self.drive,
                &self.fs,
                &self.settings,
                &self.state,
            )
            .execute(&self.account, &|_| {})
            .await
            .unwrap()
        }

        fn file(&self, path: &str) -> Option<String> {
            self.fs.read(&format!("{DIR}/{path}"))
        }

        fn files(&self) -> Vec<String> {
            self.fs.files_under(DIR)
        }
    }

    #[tokio::test]
    async fn downloads_only_the_selected_tree() {
        let w = world();
        let report = w.sync().await;

        assert_eq!(
            w.files(),
            ["Docs/Budget.html", "Docs/Sub/b.txt", "Docs/a.txt"]
        );
        assert_eq!(w.file("Docs/a.txt").as_deref(), Some("alpha"));
        assert!(
            w.file("Docs/Budget.html")
                .unwrap()
                .contains("url=https://docs.google.com/spreadsheets/d/sheet")
        );
        assert_eq!(report.downloaded, 3);
        assert_eq!(
            report.skipped,
            [(PathBuf::from("My Drive/Docs/link"), SkipReason::Shortcut)]
        );
        assert!(report.conflicts.is_empty() && report.failures.is_empty());
    }

    #[tokio::test]
    async fn second_sync_downloads_nothing() {
        let w = world();
        w.sync().await;
        let report = w.sync().await;
        assert_eq!((report.downloaded, report.moved, report.removed), (0, 0, 0));
    }

    #[tokio::test]
    async fn reports_progress() {
        let w = world();
        let updates = Mutex::new(Vec::new());
        SyncAccount::new(&w.tokens, &w.drive, &w.fs, &w.settings, &w.state)
            .execute(&w.account, &|p| updates.lock().unwrap().push(p.clone()))
            .await
            .unwrap();
        let updates = updates.into_inner().unwrap();
        let last = updates.last().unwrap();
        assert_eq!((last.files_done, last.files_total), (3, 3));
        assert_eq!(last.bytes_done, "alpha".len() as u64 + "beta".len() as u64);
        assert_eq!(updates[0].files_done, 0);
    }

    #[tokio::test]
    async fn updates_files_changed_on_drive() {
        let mut w = world();
        w.sync().await;
        w.drive.set_content("a", "alpha v2");

        let report = w.sync().await;

        assert_eq!(report.downloaded, 1);
        assert_eq!(w.file("Docs/a.txt").as_deref(), Some("alpha v2"));
    }

    #[tokio::test]
    async fn never_overwrites_local_changes() {
        let mut w = world();
        w.sync().await;
        w.fs.write_local(&format!("{DIR}/Docs/a.txt"), "my edit");
        w.drive.set_content("a", "alpha v2");

        let report = w.sync().await;

        assert_eq!(w.file("Docs/a.txt").as_deref(), Some("my edit"));
        assert_eq!(
            report.conflicts,
            [conflict("Docs/a.txt", ConflictReason::ChangedLocally)]
        );
        assert_eq!(report.downloaded, 0);
    }

    #[tokio::test]
    async fn removes_unchanged_copies_of_deleted_items_only() {
        let mut w = world();
        w.sync().await;
        w.fs.write_local(&format!("{DIR}/Docs/Sub/b.txt"), "my edit");
        w.drive.remove("a");
        w.drive.remove("b");
        w.drive.remove("sub");

        let report = w.sync().await;

        assert_eq!(w.file("Docs/a.txt"), None);
        assert_eq!(w.file("Docs/Sub/b.txt").as_deref(), Some("my edit"));
        assert_eq!(
            report.conflicts,
            [conflict("Docs/Sub/b.txt", ConflictReason::ChangedLocally)]
        );
        // "Sub" still holds the edited file, so it stays.
        assert!(w.fs.is_dir(&format!("{DIR}/Docs/Sub")));
        assert_eq!(report.removed, 1);
    }

    #[tokio::test]
    async fn follows_renames_and_moves_without_downloading() {
        let mut w = world();
        w.sync().await;
        w.drive.rename("sub", "Renamed");
        w.drive.move_to("a", "sub");

        let report = w.sync().await;

        assert_eq!(
            w.files(),
            [
                "Docs/Budget.html",
                "Docs/Renamed/a.txt",
                "Docs/Renamed/b.txt"
            ]
        );
        assert_eq!(report.downloaded, 0);
        assert_eq!(report.moved, 2);
        // And the new locations are remembered.
        let again = w.sync().await;
        assert_eq!((again.downloaded, again.moved), (0, 0));
    }

    #[tokio::test]
    async fn renamed_google_doc_gets_a_fresh_shortcut() {
        let mut w = world();
        w.sync().await;
        w.drive.rename("sheet", "Budget 2025");

        w.sync().await;

        assert_eq!(w.file("Docs/Budget.html"), None);
        assert!(
            w.file("Docs/Budget 2025.html")
                .unwrap()
                .contains("<title>Budget 2025</title>")
        );
    }

    #[tokio::test]
    async fn deselecting_keeps_local_copies() {
        let w = world();
        w.sync().await;
        w.settings
            .update(|s| s.deselect(&w.account.id, &DriveItemId::new("docs")));

        let report = w.sync().await;

        assert_eq!(report.removed, 0);
        assert_eq!(w.files().len(), 3);
        assert!(w.state.items(&w.account.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn adopts_identical_local_files_and_keeps_different_ones() {
        let w = world();
        w.fs.write_local(&format!("{DIR}/Docs/a.txt"), "alpha");
        w.fs.write_local(&format!("{DIR}/Docs/Sub/b.txt"), "not beta");

        let report = w.sync().await;

        assert_eq!(w.file("Docs/Sub/b.txt").as_deref(), Some("not beta"));
        assert_eq!(
            report.conflicts,
            [conflict("Docs/Sub/b.txt", ConflictReason::AlreadyExists)]
        );
        // a.txt was adopted (no download), the shortcut downloaded.
        assert_eq!(report.downloaded, 1);
        assert_eq!(w.sync().await.conflicts.len(), 1);
    }

    /// Copies kept after deselecting a folder are taken over again, shortcuts
    /// included, when an ancestor is selected later.
    #[tokio::test]
    async fn reselecting_adopts_kept_copies_including_shortcuts() {
        let w = world();
        w.sync().await;
        w.settings
            .update(|s| s.deselect(&w.account.id, &DriveItemId::new("docs")));
        w.sync().await; // Forgets Docs; the files stay.
        w.settings
            .update(|s| s.select(&w.account.id, DriveItemId::new("root")));

        let report = w.sync().await;

        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
        // Only Other/c.txt is new.
        assert_eq!(report.downloaded, 1);
        let tracked = w.state.items(&w.account.id).unwrap();
        assert!(tracked.iter().any(|i| i.path.ends_with("Docs/Budget.html")));
    }

    #[tokio::test]
    async fn rejects_downloads_with_wrong_checksum() {
        let mut w = world();
        w.drive.corrupt_download("a");

        let report = w.sync().await;

        assert_eq!(w.file("Docs/a.txt"), None);
        assert!(
            matches!(report.failures.as_slice(), [(path, AppError::Drive(_))] if path.ends_with("Docs/a.txt"))
        );
        assert_eq!(w.file("Docs/Sub/b.txt").as_deref(), Some("beta"));
    }

    #[tokio::test]
    async fn listing_error_aborts_before_touching_files() {
        let mut w = world();
        w.sync().await;
        w.drive.fail_listing("sub");

        let result = SyncAccount::new(&w.tokens, &w.drive, &w.fs, &w.settings, &w.state)
            .execute(&w.account, &|_| {})
            .await;

        assert!(matches!(result, Err(AppError::Drive(_))));
        assert_eq!(w.files().len(), 3);
    }

    #[tokio::test]
    async fn duplicate_names_get_suffixes() {
        let mut w = world();
        w.drive = std::mem::replace(&mut w.drive, FakeDrive::new(w.account.clone())).with_content(
            "docs",
            "a2",
            "a.txt",
            "second alpha",
        );

        w.sync().await;

        assert_eq!(w.file("Docs/a.txt").as_deref(), Some("alpha"));
        assert_eq!(w.file("Docs/a (2).txt").as_deref(), Some("second alpha"));
    }

    #[tokio::test]
    async fn nested_selection_is_synced_once_under_the_drive_path() {
        let w = world();
        w.settings.update(|s| {
            s.deselect(&w.account.id, &DriveItemId::new("docs"));
            s.select(&w.account.id, DriveItemId::new("sub"));
        });

        w.sync().await;

        // Ancestors are recreated so the path matches Drive.
        assert_eq!(w.files(), ["Docs/Sub/b.txt"]);
    }
}

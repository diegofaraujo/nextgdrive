use std::sync::Mutex;

use nextgdrive_domain::Capabilities;

use super::*;
use crate::use_cases::fakes::{
    FakeAuth, FakeDrive, FakeFileSystem, InMemoryCredentials, InMemorySettings, InMemorySyncState,
    account,
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

impl World {
    async fn try_sync(&self, deletions: Deletions) -> Result<SyncReport, AppError> {
        let options = SyncOptions {
            conflict_label: "conflict".into(),
            deletions,
        };
        SyncAccount::new(
            &self.tokens,
            &self.drive,
            &self.fs,
            &self.settings,
            &self.state,
        )
        .execute(&self.account, &options, &|_| {})
        .await
    }

    async fn sync(&self) -> SyncReport {
        let report = self.try_sync(Deletions::Ask).await.unwrap();
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        report
    }

    fn path(&self, path: &str) -> String {
        format!("{DIR}/{path}")
    }

    fn file(&self, path: &str) -> Option<String> {
        self.fs.read(&self.path(path))
    }

    fn files(&self) -> Vec<String> {
        self.fs.files_under(DIR)
    }

    fn edit(&self, path: &str, content: &str) {
        self.fs.write_local(&self.path(path), content);
    }
}

fn conflict(path: &str, reason: ConflictReason) -> Conflict {
    Conflict {
        path: PathBuf::from(format!("{DIR}/{path}")),
        reason,
    }
}

// --- Downloading -----------------------------------------------------------

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
    assert_eq!(report.uploaded, 0);
}

#[tokio::test]
async fn second_sync_does_nothing() {
    let w = world();
    w.sync().await;
    let report = w.sync().await;
    let counts = (
        report.downloaded,
        report.uploaded,
        report.moved,
        report.removed,
        report.trashed,
    );
    assert_eq!(counts, (0, 0, 0, 0, 0));
    assert!(report.conflicts.is_empty() && report.conflict_copies.is_empty());
}

#[tokio::test]
async fn reports_progress() {
    let w = world();
    let updates = Mutex::new(Vec::new());
    SyncAccount::new(&w.tokens, &w.drive, &w.fs, &w.settings, &w.state)
        .execute(&w.account, &SyncOptions::default(), &|p| {
            updates.lock().unwrap().push(p.clone())
        })
        .await
        .unwrap();
    let updates = updates.into_inner().unwrap();
    let last = updates.last().unwrap();
    assert_eq!((last.files_done, last.files_total), (3, 3));
    assert_eq!(
        last.bytes_done,
        "alpha".len() as u64 + "beta".len() as u64 + {
            w.file("Docs/Budget.html").unwrap().len() as u64
        }
    );
    assert_eq!(updates[0].files_done, 0);
}

#[tokio::test]
async fn updates_files_changed_in_drive() {
    let w = world();
    w.sync().await;
    w.drive.set_content("a", "alpha v2");

    let report = w.sync().await;

    assert_eq!(report.downloaded, 1);
    assert_eq!(w.file("Docs/a.txt").as_deref(), Some("alpha v2"));
}

#[tokio::test]
async fn removes_local_copies_of_items_deleted_in_drive() {
    let w = world();
    w.sync().await;
    w.drive.remove("sub");

    let report = w.sync().await;

    assert_eq!(w.files(), ["Docs/Budget.html", "Docs/a.txt"]);
    assert!(!w.fs.is_dir(&w.path("Docs/Sub")));
    assert_eq!(report.removed, 2);
}

#[tokio::test]
async fn follows_renames_and_moves_in_drive_without_downloading() {
    let w = world();
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
    assert_eq!((report.downloaded, report.moved), (0, 2));
    let again = w.sync().await;
    assert_eq!((again.downloaded, again.moved), (0, 0));
}

#[tokio::test]
async fn renamed_google_doc_gets_a_fresh_shortcut() {
    let w = world();
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
async fn deselected_folders_are_forgotten_but_kept() {
    let w = world();
    w.sync().await;
    w.settings
        .update(|s| s.deselect(&w.account.id, &DriveItemId::new("docs")));

    let report = w.sync().await;

    assert_eq!((report.removed, report.trashed), (0, 0));
    assert_eq!(w.files().len(), 3);
    assert!(w.state.items(&w.account.id).unwrap().is_empty());
}

#[tokio::test]
async fn reselecting_adopts_kept_copies_including_shortcuts() {
    let w = world();
    w.sync().await;
    w.settings
        .update(|s| s.deselect(&w.account.id, &DriveItemId::new("docs")));
    w.sync().await;
    w.settings
        .update(|s| s.select(&w.account.id, DriveItemId::new("root")));

    let report = w.sync().await;

    assert!(
        report.conflict_copies.is_empty(),
        "{:?}",
        report.conflict_copies
    );
    assert_eq!((report.downloaded, report.uploaded), (1, 0)); // Only Other/c.txt.
}

#[tokio::test]
async fn rejects_downloads_with_wrong_checksum() {
    let w = world();
    w.drive.corrupt_download("a");

    let report = w.try_sync(Deletions::Ask).await.unwrap();

    assert_eq!(w.file("Docs/a.txt"), None);
    assert!(
        matches!(report.failures.as_slice(), [(path, AppError::Drive(_))] if path.ends_with("Docs/a.txt"))
    );
    assert_eq!(w.file("Docs/Sub/b.txt").as_deref(), Some("beta"));
}

#[tokio::test]
async fn listing_error_aborts_before_touching_anything() {
    let w = world();
    w.sync().await;
    w.fs.delete_local(&w.path("Docs/a.txt"));
    w.drive.fail_listing("sub");

    let result = w.try_sync(Deletions::Ask).await;

    assert!(matches!(result, Err(AppError::Drive(_))));
    assert!(w.drive.trashed().is_empty());
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
async fn nested_selection_is_synced_under_the_drive_path() {
    let w = world();
    w.settings.update(|s| {
        s.deselect(&w.account.id, &DriveItemId::new("docs"));
        s.select(&w.account.id, DriveItemId::new("sub"));
    });

    w.sync().await;

    assert_eq!(w.files(), ["Docs/Sub/b.txt"]);
}

// --- Uploading -------------------------------------------------------------

#[tokio::test]
async fn uploads_local_edits() {
    let w = world();
    w.sync().await;
    w.edit("Docs/a.txt", "alpha edited here");

    let report = w.sync().await;

    assert_eq!(report.uploaded, 1);
    assert_eq!(w.drive.content("a").as_deref(), Some("alpha edited here"));
    assert_eq!(w.sync().await.uploaded, 0);
}

#[tokio::test]
async fn uploads_new_files_and_folders() {
    let w = world();
    w.sync().await;
    w.edit("Docs/new.txt", "fresh");
    w.edit("Docs/New folder/deep/x.txt", "deep");

    let report = w.sync().await;

    assert_eq!(report.uploaded, 2);
    assert_eq!(
        w.drive.find("docs", "new.txt").map(|i| i.name).as_deref(),
        Some("new.txt")
    );
    let folder = w.drive.find("docs", "New folder").unwrap();
    let deep = w.drive.find(folder.id.as_str(), "deep").unwrap();
    let x = w.drive.find(deep.id.as_str(), "x.txt").unwrap();
    assert_eq!(w.drive.content(x.id.as_str()).as_deref(), Some("deep"));
    let again = w.sync().await;
    assert_eq!((again.uploaded, again.downloaded), (0, 0));
}

#[tokio::test]
async fn never_uploads_temporary_files() {
    let w = world();
    w.sync().await;
    w.edit("Docs/.~lock.a.txt#", "lock");
    w.edit("Docs/notes.txt.swp", "swap");

    assert_eq!(w.sync().await.uploaded, 0);
}

#[tokio::test]
async fn files_outside_selected_folders_are_ignored() {
    let w = world();
    w.sync().await;
    w.edit("Other/local.txt", "not synced");

    assert_eq!(w.sync().await.uploaded, 0);
}

#[tokio::test]
async fn local_renames_and_moves_happen_in_drive() {
    let w = world();
    w.sync().await;
    w.fs.move_local(&w.path("Docs/a.txt"), &w.path("Docs/Sub/renamed.txt"));
    w.fs.move_local(&w.path("Docs/Sub"), &w.path("Docs/Sub moved"));

    let report = w.sync().await;

    assert_eq!(
        (report.uploaded, report.trashed, report.moved_in_drive),
        (0, 0, 2)
    );
    let sub = w.drive.find("docs", "Sub moved").unwrap();
    assert_eq!(sub.id.as_str(), "sub");
    // Same Drive file: its history and sharing are kept.
    assert_eq!(w.drive.find("sub", "renamed.txt").unwrap().id.as_str(), "a");
    assert_eq!(w.sync().await.moved_in_drive, 0);
}

#[tokio::test]
async fn renaming_a_shortcut_renames_the_document() {
    let w = world();
    w.sync().await;
    w.fs.move_local(
        &w.path("Docs/Budget.html"),
        &w.path("Docs/Budget 2026.html"),
    );

    w.sync().await;

    assert_eq!(
        w.drive.find("docs", "Budget 2026").unwrap().id.as_str(),
        "sheet"
    );
}

#[tokio::test]
async fn local_deletions_go_to_the_drive_trash() {
    let w = world();
    w.sync().await;
    w.fs.delete_local(&w.path("Docs/a.txt"));
    w.fs.delete_local(&w.path("Docs/Budget.html"));

    let report = w.sync().await;

    assert_eq!(report.trashed, 2);
    assert_eq!(w.drive.trashed(), ["a", "sheet"]);
    assert!(!w.drive.has("a"));
}

#[tokio::test]
async fn deleting_a_folder_trashes_it_as_one_item() {
    let w = world();
    w.sync().await;
    w.fs.delete_local(&w.path("Docs/Sub"));

    let report = w.sync().await;

    assert_eq!(w.drive.trashed(), ["sub"]);
    assert_eq!(report.trashed, 1);
    assert!(!w.drive.has("b"));
}

#[tokio::test]
async fn deleted_folder_is_kept_if_something_in_it_changed_in_drive() {
    let w = world();
    w.sync().await;
    w.fs.delete_local(&w.path("Docs/Sub"));
    w.drive.set_content("b", "beta edited by someone");

    w.sync().await;

    assert!(w.drive.trashed().is_empty());
    assert_eq!(
        w.file("Docs/Sub/b.txt").as_deref(),
        Some("beta edited by someone")
    );
}

#[tokio::test]
async fn deleted_file_comes_back_if_it_changed_in_drive() {
    let w = world();
    w.sync().await;
    w.fs.delete_local(&w.path("Docs/a.txt"));
    w.drive.set_content("a", "alpha edited by someone");

    w.sync().await;

    assert!(w.drive.trashed().is_empty());
    assert_eq!(
        w.file("Docs/a.txt").as_deref(),
        Some("alpha edited by someone")
    );
}

#[tokio::test]
async fn files_of_others_are_removed_from_my_drive_not_trashed() {
    let w = world();
    w.sync().await;
    let others = Capabilities {
        edit: true,
        trash: false,
        remove_from_my_drive: true,
    };
    w.drive.set_capabilities("a", others);
    w.fs.delete_local(&w.path("Docs/a.txt"));

    w.sync().await;

    assert!(w.drive.trashed().is_empty());
    assert_eq!(w.drive.removed_from_my_drive(), ["a"]);
}

#[tokio::test]
async fn files_that_cannot_be_deleted_are_restored() {
    let w = world();
    w.sync().await;
    let locked = Capabilities {
        edit: false,
        trash: false,
        remove_from_my_drive: false,
    };
    w.drive.set_capabilities("a", locked);
    w.fs.delete_local(&w.path("Docs/a.txt"));

    let report = w.sync().await;

    assert_eq!(w.file("Docs/a.txt").as_deref(), Some("alpha"));
    assert_eq!(
        report.conflicts,
        [conflict("Docs/a.txt", ConflictReason::CannotDelete)]
    );
}

#[tokio::test]
async fn edits_to_read_only_files_are_kept_locally() {
    let w = world();
    w.sync().await;
    let read_only = Capabilities {
        edit: false,
        ..Capabilities::default()
    };
    w.drive.set_capabilities("a", read_only);
    w.edit("Docs/a.txt", "my edit");

    let report = w.sync().await;

    assert_eq!(w.drive.content("a").as_deref(), Some("alpha"));
    assert_eq!(w.file("Docs/a.txt").as_deref(), Some("my edit"));
    assert_eq!(
        report.conflicts,
        [conflict("Docs/a.txt", ConflictReason::ReadOnly)]
    );
}

// --- Conflicts -------------------------------------------------------------

#[tokio::test]
async fn changed_on_both_sides_keeps_both_versions() {
    let w = world();
    w.sync().await;
    w.edit("Docs/a.txt", "my edit");
    w.drive.set_content("a", "their edit");

    let report = w.sync().await;

    assert_eq!(w.file("Docs/a.txt").as_deref(), Some("their edit"));
    assert_eq!(w.file("Docs/a (conflict).txt").as_deref(), Some("my edit"));
    let copy = w.drive.find("docs", "a (conflict).txt").unwrap();
    assert_eq!(
        w.drive.content(copy.id.as_str()).as_deref(),
        Some("my edit")
    );
    assert_eq!(
        report.conflict_copies,
        [PathBuf::from(w.path("Docs/a (conflict).txt"))]
    );
    let again = w.sync().await;
    assert_eq!((again.uploaded, again.downloaded), (0, 0));
}

#[tokio::test]
async fn edited_here_and_deleted_in_drive_is_uploaded_again() {
    let w = world();
    w.sync().await;
    w.edit("Docs/a.txt", "my edit");
    w.drive.remove("a");

    let report = w.sync().await;

    assert_eq!(report.uploaded, 1);
    let back = w.drive.find("docs", "a.txt").unwrap();
    assert_eq!(
        w.drive.content(back.id.as_str()).as_deref(),
        Some("my edit")
    );
}

#[tokio::test]
async fn different_untracked_file_becomes_a_conflict_copy() {
    let w = world();
    w.edit("Docs/a.txt", "already here");

    let report = w.sync().await;

    assert_eq!(w.file("Docs/a.txt").as_deref(), Some("alpha"));
    assert_eq!(
        w.file("Docs/a (conflict).txt").as_deref(),
        Some("already here")
    );
    assert_eq!(report.conflict_copies.len(), 1);
}

#[tokio::test]
async fn edited_shortcut_is_rewritten_and_the_document_untouched() {
    let w = world();
    w.sync().await;
    w.edit("Docs/Budget.html", "scribbles");

    let report = w.sync().await;

    assert_eq!(report.uploaded, 0);
    assert!(
        w.file("Docs/Budget.html")
            .unwrap()
            .contains("<title>Budget</title>")
    );
}

// --- Safety guards ---------------------------------------------------------

#[tokio::test]
async fn vanished_folder_stops_the_sync() {
    let w = world();
    w.sync().await;
    w.fs.delete_local(&w.path("Docs"));

    let result = w.try_sync(Deletions::Trash).await;

    assert!(matches!(result, Err(AppError::LocalFolderMissing(path)) if path.ends_with("Docs")));
    assert!(w.drive.trashed().is_empty());
}

/// Docs with `n` files, all synced.
async fn big_world(n: usize) -> World {
    let mut w = world();
    let mut drive = std::mem::replace(&mut w.drive, FakeDrive::new(w.account.clone()));
    for i in 0..n {
        drive = drive.with_content("docs", &format!("f{i}"), &format!("f{i}.txt"), "x");
    }
    w.drive = drive;
    w.sync().await;
    w
}

#[tokio::test]
async fn many_deletions_wait_for_confirmation() {
    let w = big_world(30).await;
    for i in 0..30 {
        w.fs.delete_local(&w.path(&format!("Docs/f{i}.txt")));
    }

    let result = w.try_sync(Deletions::Ask).await;
    assert!(matches!(result, Err(AppError::MassDeletion { count: 30 })));
    assert!(w.drive.trashed().is_empty());

    let report = w.try_sync(Deletions::Trash).await.unwrap();
    assert_eq!(report.trashed, 30);
}

#[tokio::test]
async fn many_deletions_can_be_undone() {
    let w = big_world(30).await;
    for i in 0..30 {
        w.fs.delete_local(&w.path(&format!("Docs/f{i}.txt")));
    }

    let report = w.try_sync(Deletions::Restore).await.unwrap();

    assert_eq!(report.downloaded, 30);
    assert!(w.drive.trashed().is_empty());
}

#[tokio::test]
async fn a_few_deletions_need_no_confirmation() {
    let w = big_world(30).await;
    for i in 0..MASS_DELETION_MIN {
        w.fs.delete_local(&w.path(&format!("Docs/f{i}.txt")));
    }

    let report = w.try_sync(Deletions::Ask).await.unwrap();

    assert_eq!(report.trashed, MASS_DELETION_MIN);
}

#[tokio::test]
async fn a_file_replaced_in_drive_by_another_of_the_same_name_arrives_in_one_sync() {
    let mut w = world();
    w.sync().await;
    // What `sed -i` and many editors do: the old file goes, a new one takes its name.
    w.drive.remove("a");
    w.drive = std::mem::replace(&mut w.drive, FakeDrive::new(w.account.clone())).with_content(
        "docs",
        "a2",
        "a.txt",
        "replacement",
    );

    let report = w.sync().await;

    assert_eq!(w.file("Docs/a.txt").as_deref(), Some("replacement"));
    assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    assert!(report.conflict_copies.is_empty());
}

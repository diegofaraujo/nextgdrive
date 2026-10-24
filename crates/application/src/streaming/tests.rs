use std::sync::atomic::Ordering;

use super::*;
use crate::use_cases::fakes::InMemoryStaging;
use crate::use_cases::fakes::{
    FakeAuth, FakeDrive, InMemoryBlocks, InMemoryCredentials, InMemorySettings, account,
};

type Fs<'a> = StreamingDrive<
    'a,
    FakeAuth,
    InMemoryCredentials,
    FakeDrive,
    InMemoryBlocks,
    InMemorySettings,
    InMemoryStaging,
>;

struct World {
    account: Account,
    tokens: TokenProvider<FakeAuth, InMemoryCredentials>,
    drive: FakeDrive,
    cache: InMemoryBlocks,
    settings: InMemorySettings,
    staging: InMemoryStaging,
    state: StreamState,
}

fn world(listing_ttl: Duration) -> World {
    let account = account("1", "ana@x.com");
    World {
        tokens: TokenProvider::new(
            FakeAuth::default(),
            InMemoryCredentials::with_token("1", FakeAuth::REFRESH_TOKEN),
        ),
        drive: FakeDrive::new(account.clone())
            .with_folder("root", "docs", "Docs")
            .with_content("docs", "a", "a.txt", "0123456789abcdefghij")
            .with_item(
                "docs",
                "sheet",
                "Budget",
                ItemKind::GoogleDoc {
                    url: "https://docs.google.com/spreadsheets/d/sheet".into(),
                },
            )
            .with_item("docs", "link", "link", ItemKind::Shortcut),
        account,
        cache: InMemoryBlocks::default(),
        settings: InMemorySettings::default(),
        staging: InMemoryStaging::default(),
        // Tiny blocks, so reads span several.
        state: StreamState::new(StreamConfig {
            listing_ttl,
            block_size: 8,
            readahead_blocks: 2,
            // Uploads happen as soon as asked, unless a test says otherwise.
            new_file_quiet: Duration::ZERO,
            retry_backoff: Duration::ZERO,
        }),
    }
}

impl World {
    fn fs(&self) -> Fs<'_> {
        StreamingDrive::new(
            &self.tokens,
            &self.drive,
            &self.cache,
            &self.settings,
            &self.staging,
            &self.state,
            &self.account,
            "conflict".into(),
        )
    }

    async fn path(&self, path: &[&str]) -> Entry {
        let mut dir = ROOT;
        let mut entry = None;
        for name in path {
            let found = self
                .fs()
                .lookup(dir, name)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{name}"));
            dir = found.attr.id;
            entry = Some(found);
        }
        entry.expect("non-empty path")
    }
}

fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

#[tokio::test]
async fn shows_the_drive_without_downloading_anything() {
    let w = world(Duration::from_secs(60));

    assert_eq!(names(&w.fs().list(ROOT).await.unwrap()), ["My Drive"]);
    let docs = w.path(&["My Drive", "Docs"]).await;
    let entries = w.fs().list(docs.attr.id).await.unwrap();

    assert_eq!(names(&entries), ["a.txt", "Budget.html"]);
    assert_eq!(entries[0].attr.size, 20);
    assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn reads_fetch_only_the_blocks_needed_and_only_once() {
    let w = world(Duration::from_secs(60));
    let a = w.path(&["My Drive", "Docs", "a.txt"]).await;

    // Bytes 6..10 span blocks 0 and 1.
    assert_eq!(w.fs().read(a.attr.id, 6, 4).await.unwrap(), b"6789");
    assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 2);

    assert_eq!(
        w.fs().read(a.attr.id, 0, 100).await.unwrap(),
        b"0123456789abcdefghij"
    );
    // Only the last block was new.
    assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 3);
    assert_eq!(w.fs().read(a.attr.id, 20, 5).await.unwrap(), b"");
}

#[tokio::test]
async fn sequential_reads_fetch_ahead_in_one_request() {
    let w = world(Duration::from_secs(60));
    let a = w.path(&["My Drive", "Docs", "a.txt"]).await;
    let calls = || w.drive.range_calls.load(Ordering::Relaxed);

    // The first read is not known to be sequential: one block.
    assert_eq!(w.fs().read(a.attr.id, 0, 4).await.unwrap(), b"0123");
    assert_eq!(calls(), 1);
    // Continuing where it ended: blocks 1 and 2 come in one request.
    assert_eq!(w.fs().read(a.attr.id, 4, 8).await.unwrap(), b"456789ab");
    assert_eq!(calls(), 2);
    assert_eq!(w.fs().read(a.attr.id, 12, 8).await.unwrap(), b"cdefghij");
    assert_eq!(calls(), 2, "already fetched ahead");
}

#[tokio::test]
async fn new_versions_never_get_stale_blocks() {
    let w = world(Duration::ZERO);
    let a = w.path(&["My Drive", "Docs", "a.txt"]).await;
    w.fs().read(a.attr.id, 0, 4).await.unwrap();

    w.drive.set_content("a", "NEW CONTENT");
    let again = w.path(&["My Drive", "Docs", "a.txt"]).await;

    assert_eq!(again.attr.id, a.attr.id, "same file keeps its node");
    assert_eq!(again.attr.size, 11);
    assert_eq!(w.fs().read(a.attr.id, 0, 4).await.unwrap(), b"NEW ");
}

#[tokio::test]
async fn google_docs_read_as_their_shortcut() {
    let w = world(Duration::from_secs(60));
    let doc = w.path(&["My Drive", "Docs", "Budget.html"]).await;

    let page = w.fs().read(doc.attr.id, 0, 10_000).await.unwrap();

    assert_eq!(page.len() as u64, doc.attr.size);
    assert!(
        String::from_utf8(page)
            .unwrap()
            .contains("docs.google.com/spreadsheets/d/sheet")
    );
}

#[tokio::test]
async fn renamed_items_keep_their_node() {
    let w = world(Duration::ZERO);
    let a = w.path(&["My Drive", "Docs", "a.txt"]).await;

    w.drive.rename("a", "renamed.txt");
    let docs = w.path(&["My Drive", "Docs"]).await;

    assert!(
        w.fs()
            .lookup(docs.attr.id, "a.txt")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        w.path(&["My Drive", "Docs", "renamed.txt"]).await.attr.id,
        a.attr.id
    );
}

#[tokio::test]
async fn listings_are_reused_until_they_expire() {
    let w = world(Duration::from_secs(60));
    let docs = w.path(&["My Drive", "Docs"]).await;
    w.fs().list(docs.attr.id).await.unwrap();

    w.drive.rename("a", "renamed.txt");

    assert_eq!(names(&w.fs().list(docs.attr.id).await.unwrap())[0], "a.txt");
}

// --- Writing -----------------------------------------------------------------

impl World {
    async fn docs(&self) -> NodeId {
        self.path(&["My Drive", "Docs"]).await.attr.id
    }

    async fn node(&self, name: &str) -> NodeId {
        self.path(&["My Drive", "Docs", name]).await.attr.id
    }

    async fn read_all(&self, node: NodeId) -> String {
        String::from_utf8(self.fs().read(node, 0, 1 << 20).await.unwrap()).unwrap()
    }
}

#[tokio::test]
async fn new_files_are_uploaded_when_closed() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;

    let file = w.fs().create(docs, "new.txt").await.unwrap();
    w.fs().write(file.attr.id, 0, b"hello").unwrap();
    w.fs().flush(file.attr.id).await.unwrap();
    assert!(
        w.drive.find("docs", "new.txt").is_none(),
        "new files wait for a quiet moment"
    );
    w.fs().upload_pending().await;

    let uploaded = w.drive.find("docs", "new.txt").unwrap();
    assert_eq!(
        w.drive.content(uploaded.id.as_str()).as_deref(),
        Some("hello")
    );
    assert_eq!(w.staging.count(), 0);
    // Reading it back comes from the cache.
    assert_eq!(w.read_all(file.attr.id).await, "hello");
    assert_eq!(w.drive.range_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn edits_become_a_new_version_of_the_same_file() {
    let w = world(Duration::from_secs(60));
    let a = w.node("a.txt").await;

    w.fs().open_write(a, false).await.unwrap();
    w.fs().write(a, 0, b"XYZ").unwrap();
    w.fs().flush(a).await.unwrap();

    assert_eq!(
        w.drive.content("a").as_deref(),
        Some("XYZ3456789abcdefghij")
    );
    assert_eq!(w.read_all(a).await, "XYZ3456789abcdefghij");
}

#[tokio::test]
async fn truncating_replaces_the_content() {
    let w = world(Duration::from_secs(60));
    let a = w.node("a.txt").await;

    w.fs().open_write(a, true).await.unwrap();
    w.fs().write(a, 0, b"short").unwrap();
    w.fs().flush(a).await.unwrap();

    assert_eq!(w.drive.content("a").as_deref(), Some("short"));
    assert_eq!(w.fs().attr(a).unwrap().size, 5);
}

#[tokio::test]
async fn changed_in_drive_while_editing_becomes_a_conflict_copy() {
    let w = world(Duration::from_secs(60));
    let a = w.node("a.txt").await;
    w.fs().open_write(a, true).await.unwrap();
    w.fs().write(a, 0, b"mine").unwrap();
    w.drive.set_content("a", "theirs");

    let outcome = w.fs().flush(a).await.unwrap().unwrap();

    assert_eq!(outcome.conflict_copy.as_deref(), Some("a (conflict).txt"));
    assert_eq!(w.drive.content("a").as_deref(), Some("theirs"));
    let copy = w.drive.find("docs", "a (conflict).txt").unwrap();
    assert_eq!(w.drive.content(copy.id.as_str()).as_deref(), Some("mine"));
}

#[tokio::test]
async fn failed_uploads_are_kept_and_retried_after_a_restart() {
    let mut w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    w.drive.fail_uploads.store(true, Ordering::Relaxed);
    let file = w.fs().create(docs, "offline.txt").await.unwrap();
    w.fs().write(file.attr.id, 0, b"safe").unwrap();

    let results = w.fs().upload_pending().await;
    assert!(matches!(results.as_slice(), [(_, Err(_))]), "{results:?}");
    assert_eq!(w.staging.count(), 1);

    // Restart: a new mount finds it, shows it, and uploads it.
    w.state = StreamState::new(w.state.config);
    assert_eq!(w.fs().recover().unwrap(), 1);
    assert_eq!(w.read_all(w.node("offline.txt").await).await, "safe");
    w.drive.fail_uploads.store(false, Ordering::Relaxed);
    let results = w.fs().upload_pending().await;

    assert!(results.iter().all(|(_, r)| r.is_ok()), "{results:?}");
    let uploaded = w.drive.find("docs", "offline.txt").unwrap();
    assert_eq!(
        w.drive.content(uploaded.id.as_str()).as_deref(),
        Some("safe")
    );
    assert_eq!(w.staging.count(), 0);
}

#[tokio::test]
async fn deleting_moves_to_the_trash() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    w.fs().unlink(docs, "a.txt").await.unwrap();
    w.fs().unlink(docs, "Budget.html").await.unwrap();

    assert_eq!(w.drive.trashed(), ["a", "sheet"]);
    assert!(w.fs().lookup(docs, "a.txt").await.unwrap().is_none());
}

#[tokio::test]
async fn others_items_are_removed_from_my_drive_and_locked_ones_refused() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    w.drive.set_capabilities(
        "a",
        nextgdrive_domain::Capabilities {
            edit: true,
            trash: false,
            remove_from_my_drive: true,
        },
    );
    w.fs().unlink(docs, "a.txt").await.unwrap();
    assert_eq!(w.drive.removed_from_my_drive(), ["a"]);

    w.drive.set_capabilities(
        "sheet",
        nextgdrive_domain::Capabilities {
            edit: false,
            trash: false,
            remove_from_my_drive: false,
        },
    );
    assert!(matches!(
        w.fs().unlink(docs, "Budget.html").await,
        Err(StreamError::NotPermitted)
    ));
}

#[tokio::test]
async fn folders_are_created_and_only_empty_ones_removed() {
    let w = world(Duration::from_secs(60));
    let my_drive = w.path(&["My Drive"]).await.attr.id;

    let new = w.fs().mkdir(my_drive, "New").await.unwrap();
    assert!(w.drive.find("root", "New").is_some());
    assert_eq!(w.fs().attr(new.attr.id).unwrap().kind, NodeKind::Dir);

    assert!(matches!(
        w.fs().rmdir(my_drive, "Docs").await,
        Err(StreamError::NotEmpty)
    ));
    w.fs().rmdir(my_drive, "New").await.unwrap();
    assert!(w.drive.find("root", "New").is_none());
}

#[tokio::test]
async fn saving_through_a_temporary_file_keeps_the_files_identity() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    // How gedit and others save: write a temporary file, then rename it over.
    let temp = w.fs().create(docs, ".goutputstream-ABC123").await.unwrap();
    w.fs().write(temp.attr.id, 0, b"saved").unwrap();
    w.fs().flush(temp.attr.id).await.unwrap();
    assert!(
        w.drive.find("docs", ".goutputstream-ABC123").is_none(),
        "temporary files are never uploaded"
    );

    w.fs()
        .rename(docs, ".goutputstream-ABC123", docs, "a.txt")
        .await
        .unwrap();

    assert_eq!(w.drive.content("a").as_deref(), Some("saved"));
    assert!(w.drive.trashed().is_empty());
    assert_eq!(w.read_all(w.node("a.txt").await).await, "saved");
}

#[tokio::test]
async fn renames_and_moves_happen_in_drive() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    let my_drive = w.path(&["My Drive"]).await.attr.id;

    w.fs()
        .rename(docs, "a.txt", my_drive, "moved.txt")
        .await
        .unwrap();
    w.fs()
        .rename(docs, "Budget.html", docs, "Budget 2026.html")
        .await
        .unwrap();

    assert_eq!(w.drive.find("root", "moved.txt").unwrap().id.as_str(), "a");
    // A shortcut renames its document, without the extension.
    assert_eq!(
        w.drive.find("docs", "Budget 2026").unwrap().id.as_str(),
        "sheet"
    );
}

#[tokio::test]
async fn google_doc_shortcuts_and_the_account_folder_are_not_writable() {
    let w = world(Duration::from_secs(60));
    let doc = w.node("Budget.html").await;

    assert!(matches!(
        w.fs().open_write(doc, false).await,
        Err(StreamError::NotPermitted)
    ));
    assert!(matches!(
        w.fs().create(ROOT, "x.txt").await,
        Err(StreamError::NotPermitted)
    ));
}

#[tokio::test]
async fn saving_through_an_ordinary_temporary_name_keeps_identity_too() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    // How `sed -i` saves: a temporary file with an ordinary name.
    let temp = w.fs().create(docs, "sedAB12cd").await.unwrap();
    w.fs().write(temp.attr.id, 0, b"edited by sed").unwrap();
    w.fs().flush(temp.attr.id).await.unwrap();

    w.fs()
        .rename(docs, "sedAB12cd", docs, "a.txt")
        .await
        .unwrap();

    assert_eq!(w.drive.content("a").as_deref(), Some("edited by sed"));
    assert!(w.drive.trashed().is_empty());
    assert!(w.drive.find("docs", "sedAB12cd").is_none());
}

#[tokio::test]
async fn new_files_still_changing_are_not_uploaded_yet() {
    let mut w = world(Duration::from_secs(60));
    w.state = StreamState::new(StreamConfig {
        new_file_quiet: Duration::from_secs(60),
        ..w.state.config
    });
    let docs = w.docs().await;
    let file = w.fs().create(docs, "draft.txt").await.unwrap();
    w.fs().write(file.attr.id, 0, b"draft").unwrap();
    w.fs().flush(file.attr.id).await.unwrap();

    assert!(w.fs().upload_pending().await.is_empty());
    assert!(w.drive.find("docs", "draft.txt").is_none());
    assert_eq!(
        w.read_all(file.attr.id).await,
        "draft",
        "readable meanwhile"
    );
}

#[tokio::test]
async fn edits_of_existing_files_upload_on_close() {
    let w = world(Duration::from_secs(60));
    let a = w.node("a.txt").await;
    w.fs().open_write(a, true).await.unwrap();
    w.fs().write(a, 0, b"now").unwrap();

    w.fs().flush(a).await.unwrap();

    assert_eq!(w.drive.content("a").as_deref(), Some("now"));
}

#[tokio::test]
async fn deleting_a_file_while_it_uploads_leaves_nothing_in_drive() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    let file = w.fs().create(docs, "gone.txt").await.unwrap();
    w.fs().write(file.attr.id, 0, b"short-lived").unwrap();

    // The upload pauses midway; the file is deleted meanwhile.
    let fs = w.fs();
    let (_, deleted) = tokio::join!(fs.upload_pending(), fs.unlink(docs, "gone.txt"));
    deleted.unwrap();

    assert!(
        w.drive.find("docs", "gone.txt").is_none(),
        "deleted file left in Drive"
    );
    assert_eq!(w.staging.count(), 0);
}

// --- Desktop trash -------------------------------------------------------------

/// What file managers do to "move to trash" on a mount other than home.
async fn trash_folders(w: &World) -> (NodeId, NodeId) {
    let trash = w.fs().mkdir(ROOT, ".Trash-1000").await.unwrap();
    assert!(trash.attr.private);
    let files = w.fs().mkdir(trash.attr.id, "files").await.unwrap();
    let info = w.fs().mkdir(trash.attr.id, "info").await.unwrap();
    (files.attr.id, info.attr.id)
}

#[tokio::test]
async fn moving_to_the_desktop_trash_moves_to_the_drive_trash() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    let (files, info) = trash_folders(&w).await;
    // Written under a temporary name, then renamed (as GIO does).
    let description = w.fs().create(info, "a.txt.trashinfo.TMP123").await.unwrap();
    w.fs()
        .write(description.attr.id, 0, b"[Trash Info]\nPath=...")
        .unwrap();
    w.fs()
        .rename(info, "a.txt.trashinfo.TMP123", info, "a.txt.trashinfo")
        .await
        .unwrap();
    assert!(
        w.fs()
            .lookup(info, "a.txt.trashinfo")
            .await
            .unwrap()
            .is_some()
    );

    w.fs().rename(docs, "a.txt", files, "a.txt").await.unwrap();

    assert_eq!(w.drive.trashed(), ["a"]);
    assert!(w.fs().lookup(docs, "a.txt").await.unwrap().is_none());
    // The file manager's trash view finds it.
    assert!(w.fs().lookup(files, "a.txt").await.unwrap().is_some());
    let root = w.fs().list(ROOT).await.unwrap();
    assert!(root.iter().any(|e| e.name == ".Trash-1000"));
    assert!(
        w.drive.find("docs", "a.txt.trashinfo").is_none(),
        "descriptions stay local"
    );
}

#[tokio::test]
async fn restoring_from_the_desktop_trash_restores_from_the_drive_trash() {
    let w = world(Duration::ZERO);
    let docs = w.docs().await;
    let (files, _) = trash_folders(&w).await;
    let trashed = w.fs().lookup(docs, "a.txt").await.unwrap().unwrap();
    w.fs().rename(docs, "a.txt", files, "a.txt").await.unwrap();

    w.fs().rename(files, "a.txt", docs, "a.txt").await.unwrap();

    assert!(w.drive.has("a"));
    assert!(w.fs().lookup(files, "a.txt").await.unwrap().is_none());
    // The kernel keeps using the node it had: it must still be the file.
    assert_eq!(w.read_all(trashed.attr.id).await, "0123456789abcdefghij");
    assert_eq!(
        w.read_all(w.node("a.txt").await).await,
        "0123456789abcdefghij"
    );
}

#[tokio::test]
async fn emptying_the_desktop_trash_never_deletes_from_drive() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    let (files, _) = trash_folders(&w).await;
    w.fs().rename(docs, "a.txt", files, "a.txt").await.unwrap();

    w.fs().unlink(files, "a.txt").await.unwrap();

    // Still in the Drive trash, restorable for 30 days.
    let trash = crate::use_cases::DriveTrash::new(&w.tokens, &w.drive, &w.settings);
    let entries = trash.list(&w.account).await.unwrap();
    assert!(entries.iter().any(|e| e.item.id.as_str() == "a"));
}

#[tokio::test]
async fn only_the_trash_can_be_created_in_the_account_folder() {
    let w = world(Duration::from_secs(60));
    assert!(matches!(
        w.fs().mkdir(ROOT, "Notes").await,
        Err(StreamError::NotPermitted)
    ));
    let (_, info) = trash_folders(&w).await;
    assert!(matches!(
        w.fs().mkdir(info, "x").await,
        Err(StreamError::NotPermitted)
    ));
}

#[tokio::test]
async fn trashing_over_a_placeholder_works_as_cosmic_files_does_it() {
    let w = world(Duration::from_secs(60));
    let docs = w.docs().await;
    let (files, info) = trash_folders(&w).await;
    let my_drive = w.path(&["My Drive"]).await.attr.id;
    // trash-rs: description, then an empty placeholder, then the rename over it.
    w.fs().create(info, "a.txt.trashinfo").await.unwrap();
    w.fs().create(files, "a.txt").await.unwrap();
    w.fs().rename(docs, "a.txt", files, "a.txt").await.unwrap();
    // A folder gets an empty folder as placeholder.
    w.fs().mkdir(files, "Docs").await.unwrap();
    w.fs()
        .rename(my_drive, "Docs", files, "Docs")
        .await
        .unwrap();

    assert_eq!(w.drive.trashed(), ["a", "docs"]);
    let names: Vec<_> = w
        .fs()
        .list(files)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["a.txt", "Docs"]);
}

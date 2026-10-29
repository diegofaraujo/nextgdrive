//! Sync state, and the index of streamed Drives, in SQLite databases.

mod drive_index;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use nextgdrive_application::ports::{PortError, SyncStateStore, SyncedItem, SyncedKind};
use nextgdrive_domain::{AccountId, DriveItemId};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub use drive_index::SqliteDriveIndex;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS synced_items (
    account_id      TEXT NOT NULL,
    item_id         TEXT NOT NULL,
    selection_id    TEXT NOT NULL,
    path            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    remote_modified INTEGER NOT NULL,
    remote_md5      TEXT,
    local_size      INTEGER NOT NULL,
    local_modified  INTEGER NOT NULL,
    local_inode     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, item_id)
);
CREATE TABLE IF NOT EXISTS cursors (
    account_id TEXT PRIMARY KEY,
    cursor     TEXT NOT NULL
);
";

pub struct SqliteSyncState {
    connection: Mutex<Connection>,
    /// One lock file per account, held with `flock` while syncing.
    lock_dir: PathBuf,
}

impl SqliteSyncState {
    pub fn open(path: &Path) -> Result<Self, PortError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(PortError::new)?;
        }
        let connection = Connection::open(path).map_err(PortError::new)?;
        // WAL: readers (e.g. the GUI) do not block the sync while it writes.
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(PortError::new)?;
        connection.execute_batch(SCHEMA).map_err(PortError::new)?;
        migrate(&connection).map_err(PortError::new)?;
        let lock_dir = path.with_file_name("locks");
        std::fs::create_dir_all(&lock_dir).map_err(PortError::new)?;
        Ok(Self {
            connection: Mutex::new(connection),
            lock_dir,
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, PortError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| PortError::new("state lock poisoned"))?;
        f(&connection).map_err(PortError::new)
    }
}

/// Brings databases created by older versions up to date.
fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    let has_inode = connection
        .prepare("SELECT 1 FROM pragma_table_info('synced_items') WHERE name = 'local_inode'")?
        .exists([])?;
    if !has_inode {
        // 0 means unknown; sync fills it in.
        connection.execute_batch(
            "ALTER TABLE synced_items ADD COLUMN local_inode INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    Ok(())
}

/// Nanoseconds since the Unix epoch: exact enough to compare file mtimes.
fn to_nanos(time: SystemTime) -> i64 {
    let nanos = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    i64::try_from(nanos).unwrap_or(i64::MAX)
}

fn from_nanos(nanos: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::try_from(nanos).unwrap_or(0))
}

fn kind_name(kind: SyncedKind) -> &'static str {
    match kind {
        SyncedKind::Folder => "folder",
        SyncedKind::File => "file",
        SyncedKind::GoogleDoc => "google_doc",
    }
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<SyncedItem> {
    let kind: String = row.get("kind")?;
    let kind = match kind.as_str() {
        "folder" => SyncedKind::Folder,
        "google_doc" => SyncedKind::GoogleDoc,
        _ => SyncedKind::File,
    };
    let size: i64 = row.get("local_size")?;
    Ok(SyncedItem {
        id: DriveItemId::new(row.get::<_, String>("item_id")?),
        selection: DriveItemId::new(row.get::<_, String>("selection_id")?),
        path: PathBuf::from(row.get::<_, String>("path")?),
        kind,
        remote_modified: from_nanos(row.get("remote_modified")?),
        remote_md5: row.get("remote_md5")?,
        local_size: u64::try_from(size).unwrap_or(0),
        local_modified: from_nanos(row.get("local_modified")?),
        local_inode: u64::try_from(row.get::<_, i64>("local_inode")?).unwrap_or(0),
    })
}

/// Account ids become file names; keep only safe characters.
fn lock_file_name(account: &AccountId) -> String {
    let safe: String = account
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.lock")
}

impl SyncStateStore for SqliteSyncState {
    /// The kernel releases the lock when the file is closed, even if the
    /// process crashes.
    type Lock = std::fs::File;

    fn try_lock(&self, account: &AccountId) -> Result<Option<std::fs::File>, PortError> {
        let path = self.lock_dir.join(lock_file_name(account));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(PortError::new)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(PortError::new(e)),
        }
    }

    fn cursor(&self, account: &AccountId) -> Result<Option<String>, PortError> {
        self.with(|c| {
            c.query_row(
                "SELECT cursor FROM cursors WHERE account_id = ?1",
                [account.as_str()],
                |row| row.get(0),
            )
            .optional()
        })
    }

    fn set_cursor(&self, account: &AccountId, cursor: &str) -> Result<(), PortError> {
        self.with(|c| {
            c.prepare_cached("INSERT OR REPLACE INTO cursors (account_id, cursor) VALUES (?1, ?2)")?
                .execute([account.as_str(), cursor])
                .map(drop)
        })
    }

    fn items(&self, account: &AccountId) -> Result<Vec<SyncedItem>, PortError> {
        self.with(|c| {
            let mut statement =
                c.prepare_cached("SELECT * FROM synced_items WHERE account_id = ?1")?;
            statement
                .query_map([account.as_str()], row_to_item)?
                .collect()
        })
    }

    fn upsert(&self, account: &AccountId, item: &SyncedItem) -> Result<(), PortError> {
        // Drive names are UTF-8, so local paths built from them are too.
        let path = item
            .path
            .to_str()
            .ok_or_else(|| PortError::new("path is not valid UTF-8"))?;
        self.with(|c| {
            c.prepare_cached(
                "INSERT OR REPLACE INTO synced_items
                 (account_id, item_id, selection_id, path, kind, remote_modified, remote_md5, local_size, local_modified, local_inode)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                account.as_str(),
                item.id.as_str(),
                item.selection.as_str(),
                path,
                kind_name(item.kind),
                to_nanos(item.remote_modified),
                item.remote_md5,
                i64::try_from(item.local_size).unwrap_or(i64::MAX),
                to_nanos(item.local_modified),
                i64::try_from(item.local_inode).unwrap_or(0),
            ])
            .map(drop)
        })
    }

    fn remove(&self, account: &AccountId, item: &DriveItemId) -> Result<(), PortError> {
        self.with(|c| {
            c.prepare_cached("DELETE FROM synced_items WHERE account_id = ?1 AND item_id = ?2")?
                .execute([account.as_str(), item.as_str()])
                .map(drop)
        })
    }
}

impl SqliteSyncState {
    /// Number of items synced for an account.
    pub fn count(&self, account: &AccountId) -> Result<u64, PortError> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM synced_items WHERE account_id = ?1",
                [account.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| u64::try_from(n).unwrap_or(0))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, path: &str) -> SyncedItem {
        SyncedItem {
            id: DriveItemId::new(id),
            selection: DriveItemId::new("sel"),
            path: path.into(),
            kind: SyncedKind::File,
            remote_modified: SystemTime::UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789),
            remote_md5: Some("abc".into()),
            local_size: 42,
            local_modified: SystemTime::UNIX_EPOCH + Duration::new(1_700_000_001, 987),
            local_inode: 1234,
        }
    }

    #[test]
    fn stores_items_per_account_with_exact_times() {
        let dir = tempfile::tempdir().unwrap();
        let state = SqliteSyncState::open(&dir.path().join("nested/state.sqlite3")).unwrap();
        let (ana, bob) = (AccountId::new("ana"), AccountId::new("bob"));

        state.upsert(&ana, &item("1", "My Drive/a.txt")).unwrap();
        state
            .upsert(&ana, &item("1", "My Drive/renamed.txt"))
            .unwrap();
        state.upsert(&bob, &item("2", "My Drive/b.txt")).unwrap();

        assert_eq!(
            state.items(&ana).unwrap(),
            [item("1", "My Drive/renamed.txt")]
        );
        assert_eq!(state.count(&bob).unwrap(), 1);

        state.remove(&ana, &DriveItemId::new("1")).unwrap();
        assert!(state.items(&ana).unwrap().is_empty());
    }

    #[test]
    fn lock_is_exclusive_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        // Two handles, as two processes would have.
        let (a, b) = (
            SqliteSyncState::open(&path).unwrap(),
            SqliteSyncState::open(&path).unwrap(),
        );
        let ana = AccountId::new("ana");

        let held = a.try_lock(&ana).unwrap();
        assert!(held.is_some());
        assert!(b.try_lock(&ana).unwrap().is_none());
        assert!(b.try_lock(&AccountId::new("bob")).unwrap().is_some());
        drop(held);
        assert!(b.try_lock(&ana).unwrap().is_some());
    }

    #[test]
    fn stores_change_feed_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let state = SqliteSyncState::open(&dir.path().join("state.sqlite3")).unwrap();
        let ana = AccountId::new("ana");
        assert_eq!(state.cursor(&ana).unwrap(), None);
        state.set_cursor(&ana, "41").unwrap();
        state.set_cursor(&ana, "42").unwrap();
        assert_eq!(state.cursor(&ana).unwrap().as_deref(), Some("42"));
    }

    #[test]
    fn upgrades_databases_without_inodes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE synced_items (account_id TEXT NOT NULL, item_id TEXT NOT NULL, selection_id TEXT NOT NULL,
                 path TEXT NOT NULL, kind TEXT NOT NULL, remote_modified INTEGER NOT NULL, remote_md5 TEXT,
                 local_size INTEGER NOT NULL, local_modified INTEGER NOT NULL, PRIMARY KEY (account_id, item_id));
                 INSERT INTO synced_items VALUES ('ana', '1', 'sel', 'a', 'file', 0, NULL, 1, 0);",
            )
            .unwrap();

        let state = SqliteSyncState::open(&path).unwrap();

        assert_eq!(
            state.items(&AccountId::new("ana")).unwrap()[0].local_inode,
            0
        );
        state
            .upsert(&AccountId::new("ana"), &item("2", "b"))
            .unwrap();
    }

    #[test]
    fn state_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        SqliteSyncState::open(&path)
            .unwrap()
            .upsert(&AccountId::new("ana"), &item("1", "a"))
            .unwrap();
        assert_eq!(
            SqliteSyncState::open(&path)
                .unwrap()
                .items(&AccountId::new("ana"))
                .unwrap(),
            [item("1", "a")]
        );
    }
}

//! The index of streamed accounts' Drives in a SQLite database.

use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

use nextgdrive_application::ports::{DriveIndex, IndexStatus, ItemChange, PortError};
use nextgdrive_domain::{
    AccountId, Capabilities, Drive, DriveItem, DriveItemId, DriveKind, ItemKind,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::{from_nanos, to_nanos};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS index_status (
    account_id TEXT PRIMARY KEY,
    cursor     TEXT NOT NULL,
    built      INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS index_drives (
    account_id TEXT NOT NULL,
    position   INTEGER NOT NULL,
    root_id    TEXT NOT NULL,
    -- NULL for My Drive.
    name       TEXT,
    PRIMARY KEY (account_id, position)
);
CREATE TABLE IF NOT EXISTS index_items (
    account_id TEXT NOT NULL,
    item_id    TEXT NOT NULL,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    size       INTEGER,
    md5        TEXT,
    url        TEXT,
    modified   INTEGER NOT NULL,
    -- Parent ids in Drive's order, one per line.
    parents    TEXT NOT NULL,
    can_edit   INTEGER NOT NULL,
    can_trash  INTEGER NOT NULL,
    can_remove INTEGER NOT NULL,
    PRIMARY KEY (account_id, item_id)
);
CREATE TABLE IF NOT EXISTS index_parents (
    account_id TEXT NOT NULL,
    parent_id  TEXT NOT NULL,
    item_id    TEXT NOT NULL,
    PRIMARY KEY (account_id, parent_id, item_id)
);
CREATE INDEX IF NOT EXISTS index_parents_by_item ON index_parents (account_id, item_id);
";

const ITEM_COLUMNS: &str = "i.item_id, i.name, i.kind, i.size, i.md5, i.url, i.modified, i.parents, \
     i.can_edit, i.can_trash, i.can_remove";

/// Listings read through their own connection, so they never wait for a
/// write (a rebuild writes the whole Drive in one transaction): with WAL,
/// readers see the last committed state meanwhile.
pub struct SqliteDriveIndex {
    reader: Mutex<Connection>,
    writer: Mutex<Connection>,
}

impl SqliteDriveIndex {
    pub fn open(path: &Path) -> Result<Self, PortError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(PortError::new)?;
        }
        let writer = Connection::open(path).map_err(PortError::new)?;
        writer
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(PortError::new)?;
        writer.execute_batch(SCHEMA).map_err(PortError::new)?;
        let reader = Connection::open(path).map_err(PortError::new)?;
        Ok(Self {
            reader: Mutex::new(reader),
            writer: Mutex::new(writer),
        })
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, PortError> {
        let connection = self
            .reader
            .lock()
            .map_err(|_| PortError::new("index lock poisoned"))?;
        f(&connection).map_err(PortError::new)
    }

    fn write<T>(
        &self,
        f: impl FnOnce(Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, PortError> {
        let mut connection = self
            .writer
            .lock()
            .map_err(|_| PortError::new("index lock poisoned"))?;
        let tx = connection.transaction().map_err(PortError::new)?;
        f(tx).map_err(PortError::new)
    }
}

fn kind_columns(kind: &ItemKind) -> (&'static str, Option<i64>, Option<&str>, Option<&str>) {
    match kind {
        ItemKind::Folder => ("folder", None, None, None),
        ItemKind::File { size, md5 } => (
            "file",
            Some(i64::try_from(*size).unwrap_or(i64::MAX)),
            md5.as_deref(),
            None,
        ),
        ItemKind::GoogleDoc { url } => ("google_doc", None, None, Some(url.as_str())),
        ItemKind::Shortcut => ("shortcut", None, None, None),
        ItemKind::NotDownloadable => ("not_downloadable", None, None, None),
    }
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<DriveItem> {
    let kind: String = row.get(2)?;
    let kind = match kind.as_str() {
        "folder" => ItemKind::Folder,
        "file" => ItemKind::File {
            size: u64::try_from(row.get::<_, Option<i64>>(3)?.unwrap_or(0)).unwrap_or(0),
            md5: row.get(4)?,
        },
        "google_doc" => ItemKind::GoogleDoc {
            url: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        },
        "shortcut" => ItemKind::Shortcut,
        _ => ItemKind::NotDownloadable,
    };
    let parents: String = row.get(7)?;
    Ok(DriveItem {
        id: DriveItemId::new(row.get::<_, String>(0)?),
        name: row.get(1)?,
        kind,
        modified: from_nanos(row.get(6)?),
        parents: parents.lines().map(DriveItemId::new).collect(),
        capabilities: Capabilities {
            edit: row.get(8)?,
            trash: row.get(9)?,
            remove_from_my_drive: row.get(10)?,
        },
    })
}

fn insert_item(tx: &Transaction<'_>, account: &str, item: &DriveItem) -> rusqlite::Result<()> {
    let (kind, size, md5, url) = kind_columns(&item.kind);
    let parents: Vec<&str> = item.parents.iter().map(DriveItemId::as_str).collect();
    tx.prepare_cached(
        "INSERT OR REPLACE INTO index_items
         (account_id, item_id, name, kind, size, md5, url, modified, parents,
          can_edit, can_trash, can_remove)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?
    .execute(params![
        account,
        item.id.as_str(),
        item.name,
        kind,
        size,
        md5,
        url,
        to_nanos(item.modified),
        parents.join("\n"),
        item.capabilities.edit,
        item.capabilities.trash,
        item.capabilities.remove_from_my_drive,
    ])?;
    let mut parent = tx.prepare_cached(
        "INSERT OR IGNORE INTO index_parents (account_id, parent_id, item_id) VALUES (?1, ?2, ?3)",
    )?;
    for id in &parents {
        parent.execute(params![account, id, item.id.as_str()])?;
    }
    Ok(())
}

fn delete_item(tx: &Transaction<'_>, account: &str, id: &DriveItemId) -> rusqlite::Result<()> {
    tx.prepare_cached("DELETE FROM index_items WHERE account_id = ?1 AND item_id = ?2")?
        .execute(params![account, id.as_str()])?;
    tx.prepare_cached("DELETE FROM index_parents WHERE account_id = ?1 AND item_id = ?2")?
        .execute(params![account, id.as_str()])?;
    Ok(())
}

fn get_item(
    tx: &Transaction<'_>,
    account: &str,
    id: &DriveItemId,
) -> rusqlite::Result<Option<DriveItem>> {
    tx.prepare_cached(&format!(
        "SELECT {ITEM_COLUMNS} FROM index_items i WHERE i.account_id = ?1 AND i.item_id = ?2"
    ))?
    .query_row(params![account, id.as_str()], row_to_item)
    .optional()
}

fn write_drives(tx: &Transaction<'_>, account: &str, drives: &[Drive]) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM index_drives WHERE account_id = ?1",
        params![account],
    )?;
    for (position, drive) in drives.iter().enumerate() {
        let name = match &drive.kind {
            DriveKind::MyDrive => None,
            DriveKind::Shared { name } => Some(name.as_str()),
        };
        tx.execute(
            "INSERT INTO index_drives (account_id, position, root_id, name) VALUES (?1, ?2, ?3, ?4)",
            params![
                account,
                i64::try_from(position).unwrap_or(i64::MAX),
                drive.root.as_str(),
                name
            ],
        )?;
    }
    Ok(())
}

fn delete_account(tx: &Transaction<'_>, account: &str) -> rusqlite::Result<()> {
    for table in [
        "index_status",
        "index_drives",
        "index_items",
        "index_parents",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE account_id = ?1"),
            params![account],
        )?;
    }
    Ok(())
}

impl DriveIndex for SqliteDriveIndex {
    fn status(&self, account: &AccountId) -> Result<Option<IndexStatus>, PortError> {
        self.read(|c| {
            c.query_row(
                "SELECT cursor, built FROM index_status WHERE account_id = ?1",
                params![account.as_str()],
                |row| {
                    Ok(IndexStatus {
                        cursor: row.get(0)?,
                        built: from_nanos(row.get(1)?),
                    })
                },
            )
            .optional()
        })
    }

    fn rebuild(
        &self,
        account: &AccountId,
        drives: &[Drive],
        items: &[DriveItem],
        cursor: &str,
        built: SystemTime,
    ) -> Result<(), PortError> {
        let account = account.as_str();
        self.write(|tx| {
            delete_account(&tx, account)?;
            write_drives(&tx, account, drives)?;
            for item in items {
                insert_item(&tx, account, item)?;
            }
            tx.execute(
                "INSERT INTO index_status (account_id, cursor, built) VALUES (?1, ?2, ?3)",
                params![account, cursor, to_nanos(built)],
            )?;
            tx.commit()
        })
    }

    fn apply(
        &self,
        account: &AccountId,
        changes: &[ItemChange],
        cursor: Option<&str>,
    ) -> Result<Vec<DriveItemId>, PortError> {
        let account = account.as_str();
        self.write(|tx| {
            let mut folders = Vec::new();
            for change in changes {
                let (id, new) = match change {
                    ItemChange::Changed(item) => (&item.id, Some(item)),
                    ItemChange::Removed(id) => (id, None),
                };
                let old = get_item(&tx, account, id)?;
                if old.as_ref() == new {
                    continue;
                }
                folders.extend(old.iter().chain(new).flat_map(|i| i.parents.clone()));
                delete_item(&tx, account, id)?;
                if let Some(item) = new {
                    insert_item(&tx, account, item)?;
                }
            }
            if let Some(cursor) = cursor {
                tx.execute(
                    "UPDATE index_status SET cursor = ?2 WHERE account_id = ?1",
                    params![account, cursor],
                )?;
            }
            tx.commit()?;
            folders.sort();
            folders.dedup();
            Ok(folders)
        })
    }

    fn set_drives(&self, account: &AccountId, drives: &[Drive]) -> Result<(), PortError> {
        self.write(|tx| {
            write_drives(&tx, account.as_str(), drives)?;
            tx.commit()
        })
    }

    fn drives(&self, account: &AccountId) -> Result<Vec<Drive>, PortError> {
        self.read(|c| {
            c.prepare(
                "SELECT root_id, name FROM index_drives WHERE account_id = ?1 ORDER BY position",
            )?
            .query_map(params![account.as_str()], |row| {
                let name: Option<String> = row.get(1)?;
                Ok(Drive {
                    root: DriveItemId::new(row.get::<_, String>(0)?),
                    kind: name.map_or(DriveKind::MyDrive, |name| DriveKind::Shared { name }),
                })
            })?
            .collect()
        })
    }

    fn children(
        &self,
        account: &AccountId,
        folder: &DriveItemId,
    ) -> Result<Vec<DriveItem>, PortError> {
        self.read(|c| {
            c.prepare_cached(&format!(
                "SELECT {ITEM_COLUMNS} FROM index_parents p
                 JOIN index_items i ON i.account_id = p.account_id AND i.item_id = p.item_id
                 WHERE p.account_id = ?1 AND p.parent_id = ?2"
            ))?
            .query_map(params![account.as_str(), folder.as_str()], row_to_item)?
            .collect()
        })
    }

    fn forget(&self, account: &AccountId) -> Result<(), PortError> {
        self.write(|tx| {
            delete_account(&tx, account.as_str())?;
            tx.commit()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn open() -> (tempfile::TempDir, SqliteDriveIndex) {
        let dir = tempfile::tempdir().unwrap();
        let index = SqliteDriveIndex::open(&dir.path().join("index.sqlite3")).unwrap();
        (dir, index)
    }

    fn item(id: &str, parent: &str, kind: ItemKind) -> DriveItem {
        DriveItem {
            id: DriveItemId::new(id),
            name: format!("{id}.name"),
            kind,
            modified: SystemTime::UNIX_EPOCH + Duration::from_nanos(1_500_000_123),
            parents: vec![DriveItemId::new(parent)],
            capabilities: Capabilities {
                edit: true,
                trash: false,
                remove_from_my_drive: true,
            },
        }
    }

    fn drives() -> Vec<Drive> {
        vec![
            Drive {
                root: DriveItemId::new("root"),
                kind: DriveKind::MyDrive,
            },
            Drive {
                root: DriveItemId::new("team"),
                kind: DriveKind::Shared {
                    name: "Team".into(),
                },
            },
        ]
    }

    #[test]
    fn keeps_every_item_as_drive_reported_it() {
        let (_dir, index) = open();
        let ana = AccountId::new("ana");
        assert_eq!(index.status(&ana).unwrap(), None);
        let items = [
            item("docs", "root", ItemKind::Folder),
            item(
                "a",
                "docs",
                ItemKind::File {
                    size: 20,
                    md5: Some("abc".into()),
                },
            ),
            item(
                "doc",
                "docs",
                ItemKind::GoogleDoc {
                    url: "https://docs.google.com/d".into(),
                },
            ),
            item("link", "docs", ItemKind::Shortcut),
        ];
        let built = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);

        index.rebuild(&ana, &drives(), &items, "c1", built).unwrap();

        assert_eq!(
            index.status(&ana).unwrap(),
            Some(IndexStatus {
                cursor: "c1".into(),
                built
            })
        );
        assert_eq!(index.drives(&ana).unwrap(), drives());
        let mut children = index.children(&ana, &DriveItemId::new("docs")).unwrap();
        children.sort_by(|a, b| a.id.cmp(&b.id));
        let mut expected = items[1..].to_vec();
        expected.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(children, expected);
        assert!(
            index
                .children(&AccountId::new("bob"), &DriveItemId::new("docs"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn changes_report_the_folders_they_touch() {
        let (_dir, index) = open();
        let ana = AccountId::new("ana");
        let a = item("a", "docs", ItemKind::Folder);
        index
            .rebuild(
                &ana,
                &drives(),
                std::slice::from_ref(&a),
                "c1",
                SystemTime::now(),
            )
            .unwrap();

        // Unchanged: nothing to list again.
        let same = index
            .apply(&ana, &[ItemChange::Changed(a.clone())], Some("c2"))
            .unwrap();
        assert!(same.is_empty());
        assert_eq!(index.status(&ana).unwrap().unwrap().cursor, "c2");

        // Moved: both folders.
        let moved = DriveItem {
            parents: vec![DriveItemId::new("other")],
            ..a.clone()
        };
        let folders = index
            .apply(&ana, &[ItemChange::Changed(moved)], None)
            .unwrap();
        assert_eq!(
            folders,
            [DriveItemId::new("docs"), DriveItemId::new("other")]
        );
        assert!(
            index
                .children(&ana, &DriveItemId::new("docs"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(index.status(&ana).unwrap().unwrap().cursor, "c2");

        let removed = index
            .apply(&ana, &[ItemChange::Removed(DriveItemId::new("a"))], None)
            .unwrap();
        assert_eq!(removed, [DriveItemId::new("other")]);
        assert!(
            index
                .children(&ana, &DriveItemId::new("other"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rebuilding_and_forgetting_replace_only_that_account() {
        let (_dir, index) = open();
        let (ana, bob) = (AccountId::new("ana"), AccountId::new("bob"));
        let now = SystemTime::now();
        let docs = DriveItemId::new("docs");
        index
            .rebuild(
                &ana,
                &drives(),
                &[item("a", "docs", ItemKind::Folder)],
                "c",
                now,
            )
            .unwrap();
        index
            .rebuild(
                &bob,
                &drives(),
                &[item("b", "docs", ItemKind::Folder)],
                "c",
                now,
            )
            .unwrap();

        index
            .rebuild(
                &ana,
                &drives(),
                &[item("c", "docs", ItemKind::Folder)],
                "d",
                now,
            )
            .unwrap();
        let ids = |account| -> Vec<String> {
            index
                .children(account, &docs)
                .unwrap()
                .into_iter()
                .map(|i| i.id.to_string())
                .collect()
        };
        assert_eq!(ids(&ana), ["c"]);
        assert_eq!(ids(&bob), ["b"]);

        index.forget(&ana).unwrap();
        assert_eq!(index.status(&ana).unwrap(), None);
        assert!(ids(&ana).is_empty());
        assert!(index.drives(&ana).unwrap().is_empty());
        assert_eq!(ids(&bob), ["b"]);
    }
}

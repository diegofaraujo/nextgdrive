//! The local side of the sync on a regular Linux filesystem.

use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use md5::{Digest, Md5};
use nextgdrive_application::ports::{
    DirEntry, FileReader, FileWriter, LocalFileSystem, LocalMetadata, PortError,
};
/// Files being downloaded are hidden (leading dot) and only renamed to their
/// real name once complete; sync ignores them.
pub use nextgdrive_domain::local_name::PARTIAL_SUFFIX;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};

pub struct Filesystem;

/// Watches a folder tree (inotify) while alive.
pub struct ChangeWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Calls `on_change` whenever something under `root` is created, changed,
/// moved or deleted, except for names sync ignores (such as partial downloads).
pub fn watch(
    root: &Path,
    on_change: impl Fn() + Send + 'static,
) -> Result<ChangeWatcher, PortError> {
    use notify::{EventKind, RecursiveMode, Watcher};

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
            return;
        }
        let relevant = event.paths.iter().any(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !nextgdrive_domain::local_name::is_ignored(name))
        });
        if relevant {
            on_change();
        }
    })
    .map_err(PortError::new)?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(PortError::new)?;
    Ok(ChangeWatcher { _watcher: watcher })
}

pub struct Writer {
    target: PathBuf,
    partial: PathBuf,
    file: Option<BufWriter<tokio::fs::File>>,
    hasher: Md5,
}

/// An unfinished download must not leave its partial file behind.
impl Drop for Writer {
    fn drop(&mut self) {
        if self.file.is_some() {
            let _ = fs::remove_file(&self.partial);
        }
    }
}

fn error(path: &Path, e: io::Error) -> PortError {
    PortError::new(format!("{}: {e}", path.display()))
}

fn to_metadata(meta: &fs::Metadata) -> io::Result<LocalMetadata> {
    Ok(LocalMetadata {
        is_dir: meta.is_dir(),
        size: meta.len(),
        modified: meta.modified()?,
        inode: meta.ino(),
    })
}

pub struct Reader(tokio::fs::File);

impl FileReader for Reader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, PortError> {
        // `read` may return less than asked; fill the buffer unless at the end.
        let mut filled = 0;
        while filled < buf.len() {
            match self
                .0
                .read(&mut buf[filled..])
                .await
                .map_err(PortError::new)?
            {
                0 => break,
                n => filled += n,
            }
        }
        Ok(filled)
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `dir/name` → `dir/.name.nextgdrive-part`.
fn partial_path(target: &Path) -> PathBuf {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{name}{PARTIAL_SUFFIX}"))
}

impl FileWriter for Writer {
    async fn write(&mut self, chunk: &[u8]) -> Result<(), PortError> {
        self.hasher.update(chunk);
        let file = self.file.as_mut().expect("writer used after commit");
        file.write_all(chunk)
            .await
            .map_err(|e| error(&self.partial, e))
    }

    fn md5_hex(&self) -> String {
        hex(&self.hasher.clone().finalize())
    }
}

impl LocalFileSystem for Filesystem {
    type Writer = Writer;
    type Reader = Reader;

    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>, PortError> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path).map_err(|e| error(path, e))? {
            let entry = entry.map_err(|e| error(path, e))?;
            let Ok(name) = entry.file_name().into_string() else {
                continue; // Not valid UTF-8: Drive names cannot represent it.
            };
            // Symbolic links are not followed: they could point outside the
            // synced folder, or loop.
            let meta = entry.metadata().map_err(|e| error(&entry.path(), e))?;
            if meta.file_type().is_symlink() || !(meta.is_file() || meta.is_dir()) {
                continue;
            }
            let metadata = to_metadata(&meta).map_err(|e| error(&entry.path(), e))?;
            entries.push(DirEntry { name, metadata });
        }
        Ok(entries)
    }

    async fn open_read(&self, path: &Path) -> Result<Reader, PortError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| error(path, e))?;
        Ok(Reader(file))
    }

    fn metadata(&self, path: &Path) -> Result<Option<LocalMetadata>, PortError> {
        match fs::metadata(path).and_then(|m| to_metadata(&m)) {
            Ok(meta) => Ok(Some(meta)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error(path, e)),
        }
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), PortError> {
        fs::create_dir_all(path).map_err(|e| error(path, e))
    }

    async fn begin_write(&self, path: &Path) -> Result<Writer, PortError> {
        let partial = partial_path(path);
        let file = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| error(&partial, e))?;
        Ok(Writer {
            target: path.to_owned(),
            partial,
            file: Some(BufWriter::new(file)),
            hasher: Md5::new(),
        })
    }

    async fn commit(
        &self,
        mut writer: Writer,
        modified: SystemTime,
    ) -> Result<LocalMetadata, PortError> {
        let mut file = writer.file.take().expect("writer committed twice");
        let partial = writer.partial.clone();
        let result = async {
            file.flush().await?;
            let file = file.into_inner();
            // Durable before it replaces anything.
            file.sync_all().await?;
            file.into_std().await.set_modified(modified)?;
            fs::rename(&partial, &writer.target)?;
            to_metadata(&fs::metadata(&writer.target)?)
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&partial);
        }
        result.map_err(|e| error(&writer.target, e))
    }

    async fn md5_hex(&self, path: &Path) -> Result<String, PortError> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| error(path, e))?;
        let mut hasher = Md5::new();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).await.map_err(|e| error(path, e))?;
            if read == 0 {
                return Ok(hex(&hasher.finalize()));
            }
            hasher.update(&buffer[..read]);
        }
    }

    async fn has_content(&self, path: &Path, expected: &[u8]) -> Result<bool, PortError> {
        // Cheap check first: only read files of the right size.
        match self.metadata(path)? {
            Some(meta) if !meta.is_dir && meta.size == expected.len() as u64 => {}
            _ => return Ok(false),
        }
        let content = tokio::fs::read(path).await.map_err(|e| error(path, e))?;
        Ok(content == expected)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), PortError> {
        // `rename` would silently replace an existing file.
        if fs::symlink_metadata(to).is_ok() {
            return Err(error(to, io::Error::from(ErrorKind::AlreadyExists)));
        }
        fs::rename(from, to).map_err(|e| error(from, e))
    }

    fn remove_file(&self, path: &Path) -> Result<(), PortError> {
        fs::remove_file(path).map_err(|e| error(path, e))
    }

    fn remove_dir_if_empty(&self, path: &Path) -> Result<bool, PortError> {
        match fs::remove_dir(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => Ok(false),
            Err(e) => Err(error(path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn content_appears_only_on_commit() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let mut writer = Filesystem.begin_write(&target).await.unwrap();
        writer.write(b"hello ").await.unwrap();
        writer.write(b"world").await.unwrap();
        assert!(!target.exists());
        assert_eq!(writer.md5_hex(), "5eb63bbbe01eeed093cb22bb8f5acdc3");

        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let meta = Filesystem.commit(writer, modified).await.unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "hello world");
        assert_eq!(meta.size, 11);
        assert_eq!(meta.modified, modified);
        assert!(!partial_path(&target).exists());
        assert_eq!(
            Filesystem.md5_hex(&target).await.unwrap(),
            "5eb63bbbe01eeed093cb22bb8f5acdc3"
        );
    }

    #[tokio::test]
    async fn dropped_writer_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        fs::write(&target, "old").unwrap();
        let mut writer = Filesystem.begin_write(&target).await.unwrap();
        writer.write(b"new").await.unwrap();
        drop(writer);

        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn compares_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        fs::write(&path, "same").unwrap();
        assert!(Filesystem.has_content(&path, b"same").await.unwrap());
        assert!(!Filesystem.has_content(&path, b"diff").await.unwrap());
        assert!(!Filesystem.has_content(&path, b"longer").await.unwrap());
        assert!(!Filesystem.has_content(dir.path(), b"").await.unwrap());
    }

    #[tokio::test]
    async fn lists_entries_without_symlinks_and_reads_them() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("link")).unwrap();

        let mut names: Vec<_> = Filesystem
            .list_dir(dir.path())
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.metadata.is_dir))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [("a.txt".to_owned(), false), ("sub".to_owned(), true)]
        );

        let mut reader = Filesystem
            .open_read(&dir.path().join("a.txt"))
            .await
            .unwrap();
        let mut buf = [0; 3];
        assert_eq!(reader.read(&mut buf).await.unwrap(), 3);
        assert_eq!(&buf, b"hel");
        assert_eq!(reader.read(&mut buf).await.unwrap(), 2);
        assert_eq!(reader.read(&mut buf).await.unwrap(), 0);
    }

    #[test]
    fn rename_keeps_the_inode() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, "a").unwrap();
        let before = Filesystem.metadata(&a).unwrap().unwrap().inode;
        Filesystem.rename(&a, &b).unwrap();
        assert_eq!(Filesystem.metadata(&b).unwrap().unwrap().inode, before);
    }

    #[test]
    fn watcher_reports_changes_but_not_ignored_files() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = watch(dir.path(), move || {
            let _ = tx.send(());
        })
        .unwrap();

        fs::write(dir.path().join(".a.txt.nextgdrive-part"), "partial").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "partial file triggered"
        );
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.txt"), "b").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn rename_never_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();

        assert!(Filesystem.rename(&a, &b).is_err());
        assert_eq!(fs::read_to_string(&b).unwrap(), "b");
    }

    #[test]
    fn removes_only_empty_directories() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("f"), "").unwrap();

        assert!(!Filesystem.remove_dir_if_empty(&sub).unwrap());
        fs::remove_file(sub.join("f")).unwrap();
        assert!(Filesystem.remove_dir_if_empty(&sub).unwrap());
        assert_eq!(Filesystem.metadata(&sub).unwrap(), None);
    }
}

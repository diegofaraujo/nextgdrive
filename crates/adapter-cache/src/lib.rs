//! Block cache for streamed files, on disk under the XDG cache directory.
//!
//! Layout: `<root>/<file id>/<version>/<block index>`. Reading a block
//! refreshes its modification time, which eviction uses to drop what was
//! used least recently once the cache grows past its limit.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use nextgdrive_application::ports::{BlockCache, BlockKey, PortError};

pub struct DiskBlockCache {
    root: PathBuf,
    limit: u64,
    /// Bytes on disk, kept up to date without rescanning.
    used: AtomicU64,
    /// Only one eviction at a time.
    evicting: Mutex<()>,
}

/// File ids and versions become path components: keep only safe characters.
fn component(value: &str) -> String {
    let safe: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() { "_".into() } else { safe }
}

fn error(path: &Path, e: io::Error) -> PortError {
    PortError::new(format!("{}: {e}", path.display()))
}

/// Every block file under `root`, with its size and last use.
fn blocks(root: &Path) -> Vec<(SystemTime, u64, PathBuf)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((used, meta.len(), entry.path()));
            }
        }
    }
    found
}

impl DiskBlockCache {
    /// `limit` is in bytes; the cache is trimmed to 90% of it when exceeded.
    pub fn open(root: impl Into<PathBuf>, limit: u64) -> Result<Self, PortError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| error(&root, e))?;
        let used = blocks(&root).iter().map(|(_, size, _)| size).sum();
        Ok(Self {
            root,
            limit,
            used: AtomicU64::new(used),
            evicting: Mutex::new(()),
        })
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    fn path(&self, key: &BlockKey) -> PathBuf {
        self.root
            .join(component(key.file.as_str()))
            .join(component(&key.version))
            .join(key.index.to_string())
    }

    fn evict(&self) {
        let Ok(_guard) = self.evicting.try_lock() else {
            return;
        };
        if self.used() <= self.limit {
            return;
        }
        let target = self.limit / 10 * 9;
        let mut all = blocks(&self.root);
        all.sort();
        for (_, size, path) in all {
            if self.used() <= target {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                self.used.fetch_sub(size, Ordering::Relaxed);
                // Drop now-empty version and file folders; failure just means not empty.
                for dir in path.ancestors().skip(1).take(2) {
                    let _ = fs::remove_dir(dir);
                }
            }
        }
    }
}

impl BlockCache for DiskBlockCache {
    fn get(&self, key: &BlockKey) -> Result<Option<Vec<u8>>, PortError> {
        let path = self.path(key);
        match fs::read(&path) {
            Ok(data) => {
                // Mark as recently used; a failure only makes eviction less precise.
                if let Ok(file) = fs::File::options().write(true).open(&path) {
                    let _ = file.set_modified(SystemTime::now());
                }
                Ok(Some(data))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error(&path, e)),
        }
    }

    fn put(&self, key: &BlockKey, data: &[u8]) -> Result<(), PortError> {
        let path = self.path(key);
        let dir = path.parent().expect("block paths have a parent");
        fs::create_dir_all(dir).map_err(|e| error(dir, e))?;
        let previous = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // Written aside, then renamed: a crash never leaves a cut-off block.
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, data).map_err(|e| error(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| error(&path, e))?;
        self.used
            .fetch_sub(previous.min(self.used()), Ordering::Relaxed);
        self.used.fetch_add(data.len() as u64, Ordering::Relaxed);
        if self.used() > self.limit {
            self.evict();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nextgdrive_domain::DriveItemId;

    use super::*;

    fn key(file: &str, index: u64) -> BlockKey {
        BlockKey {
            file: DriveItemId::new(file),
            version: "md5".into(),
            index,
        }
    }

    #[test]
    fn stores_and_returns_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskBlockCache::open(dir.path(), 1 << 20).unwrap();

        assert_eq!(cache.get(&key("a", 0)).unwrap(), None);
        cache.put(&key("a", 0), b"hello").unwrap();
        assert_eq!(
            cache.get(&key("a", 0)).unwrap().as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(cache.used(), 5);

        // Reopening finds what is on disk.
        assert_eq!(DiskBlockCache::open(dir.path(), 1 << 20).unwrap().used(), 5);
    }

    #[test]
    fn evicts_least_recently_used_blocks_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskBlockCache::open(dir.path(), 100).unwrap();
        cache.put(&key("old", 0), &[0; 40]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        cache.put(&key("used", 0), &[0; 40]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        // Reading "old" makes it the most recently used.
        cache.get(&key("old", 0)).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        cache.put(&key("new", 0), &[0; 40]).unwrap();

        assert!(cache.used() <= 90, "{}", cache.used());
        assert!(cache.get(&key("used", 0)).unwrap().is_none());
        assert!(cache.get(&key("old", 0)).unwrap().is_some());
        assert!(cache.get(&key("new", 0)).unwrap().is_some());
    }

    #[test]
    fn keys_cannot_escape_the_cache_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskBlockCache::open(dir.path().join("cache"), 1 << 20).unwrap();
        let evil = BlockKey {
            file: DriveItemId::new("../../etc"),
            version: "../x".into(),
            index: 0,
        };
        cache.put(&evil, b"x").unwrap();
        assert!(cache.path(&evil).starts_with(dir.path().join("cache")));
        assert!(!dir.path().join("etc").exists());
    }
}

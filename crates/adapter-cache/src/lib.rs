//! Block cache for streamed files, on disk under the XDG cache directory.
//!
//! Layout: `<root>/<file id>/<version>/<block index>`. Reading a block
//! refreshes its modification time, which eviction uses to drop what was
//! used least recently once the cache grows past its limit.

use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use nextgdrive_application::ports::{BlockCache, BlockKey, PortError};
use nextgdrive_domain::DriveItemId;

pub struct DiskBlockCache {
    root: PathBuf,
    limit: AtomicU64,
    /// Bytes on disk, kept up to date without rescanning.
    used: AtomicU64,
    /// Only one eviction at a time.
    evicting: Mutex<()>,
    /// `<file>/<version>` folders of files available offline: never evicted.
    pinned: Mutex<HashSet<PathBuf>>,
    /// Bytes of pinned blocks, as of the last eviction scan. They do not
    /// count toward the limit.
    pinned_bytes: AtomicU64,
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
            limit: AtomicU64::new(limit),
            used: AtomicU64::new(used),
            evicting: Mutex::new(()),
            pinned: Mutex::default(),
            pinned_bytes: AtomicU64::new(0),
        })
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Takes effect at once: lowering it evicts right away.
    pub fn set_limit(&self, limit: u64) {
        self.limit.store(limit, Ordering::Relaxed);
        self.evict();
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    fn path(&self, key: &BlockKey) -> PathBuf {
        self.version_dir(&key.file, &key.version)
            .join(key.index.to_string())
    }

    fn version_dir(&self, file: &DriveItemId, version: &str) -> PathBuf {
        self.root
            .join(component(file.as_str()))
            .join(component(version))
    }

    /// Bytes that count toward the limit.
    fn evictable(&self) -> u64 {
        self.used()
            .saturating_sub(self.pinned_bytes.load(Ordering::Relaxed))
    }

    fn evict(&self) {
        let Ok(_guard) = self.evicting.try_lock() else {
            return;
        };
        let pinned = self.pinned.lock().map(|p| p.clone()).unwrap_or_default();
        let is_pinned = |path: &Path| path.parent().is_some_and(|dir| pinned.contains(dir));
        let mut all = blocks(&self.root);
        // Recounted: another handle (e.g. clearing from the app) may have
        // removed blocks meanwhile.
        self.used
            .store(all.iter().map(|(_, size, _)| size).sum(), Ordering::Relaxed);
        let pinned_bytes = all
            .iter()
            .filter(|(_, _, p)| is_pinned(p))
            .map(|(_, size, _)| size)
            .sum();
        self.pinned_bytes.store(pinned_bytes, Ordering::Relaxed);
        if self.evictable() <= self.limit() {
            return;
        }
        let target = self.limit() / 10 * 9;
        all.retain(|(_, _, path)| !is_pinned(path));
        all.sort();
        for (_, size, path) in all {
            if self.evictable() <= target {
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
        if self.evictable() > self.limit() {
            self.evict();
        }
        Ok(())
    }

    fn usage(&self) -> u64 {
        let used = blocks(&self.root).iter().map(|(_, size, _)| size).sum();
        self.used.store(used, Ordering::Relaxed);
        used
    }

    fn clear_except(&self, keep: &[(DriveItemId, String)]) -> Result<u64, PortError> {
        let keep: HashSet<PathBuf> = keep
            .iter()
            .map(|(file, version)| self.version_dir(file, version))
            .collect();
        let mut freed = 0;
        for (_, size, path) in blocks(&self.root) {
            if path.parent().is_some_and(|dir| keep.contains(dir)) {
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => {
                    freed += size;
                    for dir in path.ancestors().skip(1).take(2) {
                        let _ = fs::remove_dir(dir);
                    }
                }
                // Gone already (evicted meanwhile): fine.
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(error(&path, e)),
            }
        }
        self.used
            .fetch_sub(freed.min(self.used()), Ordering::Relaxed);
        Ok(freed)
    }

    fn contains(&self, key: &BlockKey) -> bool {
        self.path(key).is_file()
    }

    fn pin(&self, files: &[(DriveItemId, String)]) {
        let dirs = files
            .iter()
            .map(|(file, version)| self.version_dir(file, version))
            .collect();
        if let Ok(mut pinned) = self.pinned.lock() {
            *pinned = dirs;
        }
        // Recount what is pinned, and trim if unpinning left too much.
        self.evict();
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
    fn pinned_files_are_never_evicted_and_do_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskBlockCache::open(dir.path(), 100).unwrap();
        cache.put(&key("offline", 0), &[0; 80]).unwrap();
        cache.pin(&[(DriveItemId::new("offline"), "md5".into())]);
        std::thread::sleep(Duration::from_millis(20));

        // 80 pinned + 90 more is past the limit only if pinned counted.
        cache.put(&key("a", 0), &[0; 45]).unwrap();
        cache.put(&key("b", 0), &[0; 45]).unwrap();
        assert!(cache.contains(&key("a", 0)) && cache.contains(&key("b", 0)));

        cache.put(&key("c", 0), &[0; 45]).unwrap();
        assert!(cache.contains(&key("offline", 0)), "pinned blocks stay");
        assert!(!cache.contains(&key("a", 0)), "oldest unpinned block goes");

        // Unpinned, it is an ordinary (old) block again.
        cache.pin(&[]);
        assert!(!cache.contains(&key("offline", 0)));
    }

    #[test]
    fn clears_all_but_the_kept_files_and_follows_a_new_limit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskBlockCache::open(dir.path(), 1000).unwrap();
        cache.put(&key("keep", 0), &[0; 10]).unwrap();
        cache.put(&key("drop", 0), &[0; 20]).unwrap();
        cache.put(&key("drop", 1), &[0; 30]).unwrap();
        assert_eq!(cache.usage(), 60);

        let freed = cache
            .clear_except(&[(DriveItemId::new("keep"), "md5".into())])
            .unwrap();

        assert_eq!(freed, 50);
        assert_eq!(cache.usage(), 10);
        assert!(cache.contains(&key("keep", 0)));

        cache.put(&key("x", 0), &[0; 40]).unwrap();
        cache.set_limit(30);
        assert!(cache.usage() <= 30, "{}", cache.usage());
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

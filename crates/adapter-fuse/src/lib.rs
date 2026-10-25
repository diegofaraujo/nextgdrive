//! FUSE filesystem for streaming: translates kernel requests into calls on a
//! [`StreamBackend`], each answered from a task on the Tokio runtime so a slow
//! download or upload never holds up other requests.

mod fusermount;

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fuser::{
    BsdFileFlags, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
    TimeOrNow, WriteFlags,
};
use nextgdrive_application::streaming::{Entry, NodeAttr, NodeId, NodeKind, StreamError};
use nextgdrive_domain::StorageQuota;

/// How long the kernel may reuse attributes and names before asking again.
const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u32 = 4096;
const O_ACCMODE: i32 = 0o3;
const O_TRUNC: i32 = 0o1000;

/// What the filesystem needs from the application.
pub trait StreamBackend: Send + Sync + 'static {
    fn attr(&self, id: NodeId) -> Option<NodeAttr>;
    fn list(&self, dir: NodeId) -> impl Future<Output = Result<Vec<Entry>, StreamError>> + Send;
    fn lookup(
        &self,
        dir: NodeId,
        name: &str,
    ) -> impl Future<Output = Result<Option<Entry>, StreamError>> + Send;
    fn read(
        &self,
        id: NodeId,
        offset: u64,
        size: u32,
    ) -> impl Future<Output = Result<Vec<u8>, StreamError>> + Send;
    fn create(
        &self,
        dir: NodeId,
        name: &str,
    ) -> impl Future<Output = Result<Entry, StreamError>> + Send;
    fn open_write(
        &self,
        id: NodeId,
        truncate: bool,
    ) -> impl Future<Output = Result<(), StreamError>> + Send;
    fn write(&self, id: NodeId, offset: u64, data: &[u8]) -> Result<u32, StreamError>;
    fn set_len(&self, id: NodeId, len: u64)
    -> impl Future<Output = Result<(), StreamError>> + Send;
    /// A program closed (or synced) the file: upload what changed.
    fn flush(&self, id: NodeId) -> impl Future<Output = Result<(), StreamError>> + Send;
    fn mkdir(
        &self,
        dir: NodeId,
        name: &str,
    ) -> impl Future<Output = Result<Entry, StreamError>> + Send;
    fn unlink(
        &self,
        dir: NodeId,
        name: &str,
    ) -> impl Future<Output = Result<(), StreamError>> + Send;
    fn rmdir(
        &self,
        dir: NodeId,
        name: &str,
    ) -> impl Future<Output = Result<(), StreamError>> + Send;
    fn rename(
        &self,
        dir: NodeId,
        name: &str,
        new_dir: NodeId,
        new_name: &str,
    ) -> impl Future<Output = Result<(), StreamError>> + Send;
    /// For `df` and file managers' free-space display.
    fn quota(&self) -> impl Future<Output = Option<StorageQuota>> + Send;
    /// Called when a request fails unexpectedly, for logs.
    fn report(&self, error: &StreamError);
}

/// Owner of every file and folder: whoever owns the mount point.
#[derive(Clone, Copy)]
struct Owner {
    uid: u32,
    gid: u32,
}

impl Owner {
    fn file_attr(self, attr: &NodeAttr) -> FileAttr {
        let (kind, perm, nlink) = match attr.kind {
            // The desktop trash must be private to its owner.
            NodeKind::Dir if attr.private => (FileType::Directory, 0o700, 2),
            NodeKind::Dir => (FileType::Directory, 0o755, 2),
            NodeKind::File => (FileType::RegularFile, 0o644, 1),
        };
        FileAttr {
            ino: INodeNo(attr.id),
            size: attr.size,
            blocks: attr.size.div_ceil(512),
            atime: attr.modified,
            mtime: attr.modified,
            ctime: attr.modified,
            crtime: attr.modified,
            kind,
            perm,
            nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }
}

fn errno<B: StreamBackend>(backend: &B, error: &StreamError) -> Errno {
    match error {
        StreamError::NotFound => Errno::ENOENT,
        StreamError::NotPermitted => Errno::EACCES,
        StreamError::NotEmpty => Errno::ENOTEMPTY,
        StreamError::IsDir => Errno::EISDIR,
        StreamError::NotDir => Errno::ENOTDIR,
        StreamError::Failed(_) => {
            backend.report(error);
            Errno::EIO
        }
    }
}

fn utf8(name: &OsStr) -> Result<String, Errno> {
    name.to_str().map(str::to_owned).ok_or(Errno::EINVAL)
}

struct StreamFs<B> {
    backend: Arc<B>,
    runtime: tokio::runtime::Handle,
    owner: Owner,
}

impl<B: StreamBackend> StreamFs<B> {
    /// Runs `task` on the runtime; it answers the request itself.
    fn spawn<F>(&self, task: impl FnOnce(Arc<B>, Owner) -> F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(task(self.backend.clone(), self.owner));
    }
}

impl<B: StreamBackend> Filesystem for StreamFs<B> {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = match utf8(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        self.spawn(|backend, owner| async move {
            match backend.lookup(parent.0, &name).await {
                Ok(Some(entry)) => reply.entry(&TTL, &owner.file_attr(&entry.attr), Generation(0)),
                Ok(None) => reply.error(Errno::ENOENT),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.backend.attr(ino.0) {
            Some(attr) => reply.attr(&TTL, &self.owner.file_attr(&attr)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        // Only the size matters; owners, modes and times are Drive's.
        self.spawn(|backend, owner| async move {
            if let Some(size) = size
                && let Err(e) = backend.set_len(ino.0, size).await
            {
                return reply.error(errno(&*backend, &e));
            }
            match backend.attr(ino.0) {
                Some(attr) => reply.attr(&TTL, &owner.file_attr(&attr)),
                None => reply.error(Errno::ENOENT),
            }
        });
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let writing = flags.0 & O_ACCMODE != 0;
        let truncate = flags.0 & O_TRUNC != 0;
        self.spawn(|backend, _| async move {
            match backend.attr(ino.0) {
                None => return reply.error(Errno::ENOENT),
                Some(attr) if attr.kind == NodeKind::Dir => return reply.error(Errno::EISDIR),
                Some(_) => {}
            }
            if writing && let Err(e) = backend.open_write(ino.0, truncate).await {
                return reply.error(errno(&*backend, &e));
            }
            reply.opened(FileHandle(0), FopenFlags::empty());
        });
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.spawn(|backend, _| async move {
            match backend.read(ino.0, offset, size).await {
                Ok(data) => reply.data(&data),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        // Local and fast: answered right away, keeping writes in order.
        match self.backend.write(ino.0, offset, data) {
            Ok(written) => reply.written(written),
            Err(e) => reply.error(errno(&*self.backend, &e)),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        self.spawn(|backend, _| async move {
            match backend.flush(ino.0).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn fsync(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        self.spawn(|backend, _| async move {
            match backend.flush(ino.0).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name = match utf8(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        self.spawn(|backend, owner| async move {
            match backend.create(parent.0, &name).await {
                Ok(entry) => reply.created(
                    &TTL,
                    &owner.file_attr(&entry.attr),
                    Generation(0),
                    FileHandle(0),
                    FopenFlags::empty(),
                ),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name = match utf8(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        self.spawn(|backend, owner| async move {
            match backend.mkdir(parent.0, &name).await {
                Ok(entry) => reply.entry(&TTL, &owner.file_attr(&entry.attr), Generation(0)),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match utf8(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        self.spawn(|backend, _| async move {
            match backend.unlink(parent.0, &name).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match utf8(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        self.spawn(|backend, _| async move {
            match backend.rmdir(parent.0, &name).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        // RENAME_NOREPLACE / RENAME_EXCHANGE are not supported.
        if !flags.is_empty() {
            return reply.error(Errno::EINVAL);
        }
        let (name, newname) = match (utf8(name), utf8(newname)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return reply.error(e),
        };
        self.spawn(|backend, _| async move {
            match backend.rename(parent.0, &name, newparent.0, &newname).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(errno(&*backend, &e)),
            }
        });
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        self.spawn(|backend, _| async move {
            let entries = match backend.list(ino.0).await {
                Ok(entries) => entries,
                Err(e) => return reply.error(errno(&*backend, &e)),
            };
            // "." and ".." first; offsets let the kernel resume a listing.
            let dots = [
                (ino.0, FileType::Directory, "."),
                (ino.0, FileType::Directory, ".."),
            ];
            let all = dots.into_iter().chain(entries.iter().map(|e| {
                let kind = match e.attr.kind {
                    NodeKind::Dir => FileType::Directory,
                    NodeKind::File => FileType::RegularFile,
                };
                (e.attr.id, kind, e.name.as_str())
            }));
            for (index, (id, kind, name)) in all
                .enumerate()
                .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            {
                if reply.add(INodeNo(id), index as u64 + 1, kind, name) {
                    break; // Buffer full; the kernel asks for the rest.
                }
            }
            reply.ok();
        });
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        self.spawn(|backend, _| async move {
            let (total, free) = match backend.quota().await {
                Some(StorageQuota {
                    usage,
                    limit: Some(limit),
                }) => (limit, limit.saturating_sub(usage)),
                // Unlimited or unknown: report plenty, so copies are not refused.
                _ => (u64::MAX / 2, u64::MAX / 4),
            };
            let block = u64::from(BLOCK_SIZE);
            reply.statfs(
                total / block,
                free / block,
                free / block,
                0,
                0,
                BLOCK_SIZE,
                255,
                BLOCK_SIZE,
            );
        });
    }
}

/// A mounted filesystem; dropping it unmounts.
pub struct Mount {
    /// Set when mounted through `FUSERMOUNT_PATH`, which fuser does not know
    /// about, so unmounting is ours.
    helper: Option<(OsString, PathBuf)>,
    _session: fuser::BackgroundSession,
}

impl Drop for Mount {
    /// Unmounting first ends the session's loop, so dropping it then returns.
    fn drop(&mut self) {
        if let Some((helper, mountpoint)) = &self.helper {
            fusermount::unmount(helper, mountpoint);
        }
    }
}

/// The mount options below, as `fusermount3 -o` takes them.
const HELPER_OPTIONS: &str =
    "fsname=nextgdrive,subtype=nextgdrive,default_permissions,nosuid,nodev";

/// Mounts `backend` at `mountpoint` (an existing, empty folder). Requests are
/// answered on `runtime`.
pub fn mount<B: StreamBackend>(
    backend: Arc<B>,
    mountpoint: &Path,
    runtime: tokio::runtime::Handle,
) -> io::Result<Mount> {
    let owner = std::fs::metadata(mountpoint)?;
    let fs = StreamFs {
        backend,
        runtime,
        owner: Owner {
            uid: owner.uid(),
            gid: owner.gid(),
        },
    };
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName("nextgdrive".into()),
        MountOption::Subtype("nextgdrive".into()),
        MountOption::DefaultPermissions,
        MountOption::NoSuid,
        MountOption::NoDev,
    ];
    if let Some(helper) = std::env::var_os("FUSERMOUNT_PATH") {
        let fd = fusermount::mount(&helper, mountpoint, HELPER_OPTIONS)?;
        let acl = config.acl;
        let session = fuser::Session::from_fd(fs, fd, acl, config).and_then(|s| s.spawn());
        let helper = (helper, mountpoint.to_owned());
        return match session {
            Ok(session) => Ok(Mount {
                helper: Some(helper),
                _session: session,
            }),
            Err(e) => {
                fusermount::unmount(&helper.0, &helper.1);
                Err(e)
            }
        };
    }
    let session = fuser::spawn_mount(fs, mountpoint, &config)?;
    Ok(Mount {
        helper: None,
        _session: session,
    })
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use nextgdrive_application::streaming::ROOT;

    use super::*;

    const HELLO: NodeId = 2;

    /// A folder with one file, `hello.txt`.
    struct OneFile;

    impl OneFile {
        fn hello() -> Entry {
            Entry {
                name: "hello.txt".into(),
                attr: NodeAttr {
                    id: HELLO,
                    kind: NodeKind::File,
                    size: 5,
                    modified: UNIX_EPOCH,
                    private: false,
                },
            }
        }
    }

    impl StreamBackend for OneFile {
        fn attr(&self, id: NodeId) -> Option<NodeAttr> {
            match id {
                ROOT => Some(NodeAttr {
                    id: ROOT,
                    kind: NodeKind::Dir,
                    size: 0,
                    modified: UNIX_EPOCH,
                    private: false,
                }),
                HELLO => Some(Self::hello().attr),
                _ => None,
            }
        }
        async fn list(&self, _: NodeId) -> Result<Vec<Entry>, StreamError> {
            Ok(vec![Self::hello()])
        }
        async fn lookup(&self, _: NodeId, name: &str) -> Result<Option<Entry>, StreamError> {
            Ok((name == "hello.txt").then(Self::hello))
        }
        async fn read(&self, _: NodeId, offset: u64, size: u32) -> Result<Vec<u8>, StreamError> {
            let data = b"hello";
            let start = usize::try_from(offset)
                .unwrap_or(usize::MAX)
                .min(data.len());
            let end = (start + size as usize).min(data.len());
            Ok(data[start..end].to_vec())
        }
        async fn create(&self, _: NodeId, _: &str) -> Result<Entry, StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn open_write(&self, _: NodeId, _: bool) -> Result<(), StreamError> {
            Err(StreamError::NotPermitted)
        }
        fn write(&self, _: NodeId, _: u64, _: &[u8]) -> Result<u32, StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn set_len(&self, _: NodeId, _: u64) -> Result<(), StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn flush(&self, _: NodeId) -> Result<(), StreamError> {
            Ok(())
        }
        async fn mkdir(&self, _: NodeId, _: &str) -> Result<Entry, StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn unlink(&self, _: NodeId, _: &str) -> Result<(), StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn rmdir(&self, _: NodeId, _: &str) -> Result<(), StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn rename(&self, _: NodeId, _: &str, _: NodeId, _: &str) -> Result<(), StreamError> {
            Err(StreamError::NotPermitted)
        }
        async fn quota(&self) -> Option<StorageQuota> {
            None
        }
        fn report(&self, _: &StreamError) {}
    }

    fn is_mounted(dir: &Path) -> bool {
        let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let dir = dir.to_string_lossy();
        mounts
            .lines()
            .any(|line| line.split(' ').nth(4) == Some(&*dir))
    }

    /// Needs FUSE (`fusermount3`). Run inside the Flatpak to check mounting
    /// from the sandbox: `flatpak run --command=<this test binary>
    /// io.github.diegofaraujo.NextGDrive --ignored`, which sets FUSERMOUNT_PATH.
    #[test]
    #[ignore = "needs FUSE; run with --ignored"]
    fn mounts_reads_and_unmounts() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        // In the home folder: the same path inside a sandbox and on the host.
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let dir = tempfile::tempdir_in(home).unwrap();

        let mounted = mount(Arc::new(OneFile), dir.path(), runtime.handle().clone()).unwrap();
        assert!(is_mounted(dir.path()));
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["hello.txt"]);
        assert_eq!(
            std::fs::read(dir.path().join("hello.txt")).unwrap(),
            b"hello"
        );

        drop(mounted);
        assert!(!is_mounted(dir.path()));
    }
}

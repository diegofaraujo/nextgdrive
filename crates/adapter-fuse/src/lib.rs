//! FUSE filesystem for streaming: translates kernel requests into calls on a
//! [`StreamBackend`], each answered from a task on the Tokio runtime so a slow
//! download never holds up other requests. Read-only for now.

use std::ffi::OsStr;
use std::future::Future;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
    LockOwner, MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    ReplyStatfs, Request,
};
use nextgdrive_application::AppError;
use nextgdrive_application::streaming::{Entry, NodeAttr, NodeId, NodeKind};
use nextgdrive_domain::StorageQuota;

/// How long the kernel may reuse attributes and names before asking again.
const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u32 = 4096;

/// What the filesystem needs from the application.
pub trait StreamBackend: Send + Sync + 'static {
    fn attr(&self, id: NodeId) -> Option<NodeAttr>;
    fn list(&self, dir: NodeId) -> impl Future<Output = Result<Vec<Entry>, AppError>> + Send;
    fn lookup(
        &self,
        dir: NodeId,
        name: &str,
    ) -> impl Future<Output = Result<Option<Entry>, AppError>> + Send;
    fn read(
        &self,
        id: NodeId,
        offset: u64,
        size: u32,
    ) -> impl Future<Output = Result<Vec<u8>, AppError>> + Send;
    /// For `df` and file managers' free-space display.
    fn quota(&self) -> impl Future<Output = Option<StorageQuota>> + Send;
    /// Called when a request fails, for logs.
    fn report(&self, error: &AppError);
}

struct StreamFs<B> {
    backend: Arc<B>,
    runtime: tokio::runtime::Handle,
    owner: Owner,
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
            NodeKind::Dir => (FileType::Directory, 0o555, 2),
            NodeKind::File => (FileType::RegularFile, 0o444, 1),
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

impl<B: StreamBackend> StreamFs<B> {
    fn failed(backend: &B, error: &AppError) -> Errno {
        backend.report(error);
        Errno::EIO
    }
}

impl<B: StreamBackend> Filesystem for StreamFs<B> {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(name) = name.to_str().map(str::to_owned) else {
            return reply.error(Errno::ENOENT);
        };
        let backend = self.backend.clone();
        let owner = self.owner;
        self.runtime.spawn(async move {
            match backend.lookup(parent.0, &name).await {
                Ok(Some(entry)) => reply.entry(&TTL, &owner.file_attr(&entry.attr), Generation(0)),
                Ok(None) => reply.error(Errno::ENOENT),
                Err(e) => reply.error(Self::failed(&backend, &e)),
            }
        });
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.backend.attr(ino.0) {
            Some(attr) => reply.attr(&TTL, &self.owner.file_attr(&attr)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        if flags.0 & libc_access_mode() != 0 {
            return reply.error(Errno::EROFS);
        }
        match self.backend.attr(ino.0) {
            Some(attr) if attr.kind == NodeKind::File => {
                reply.opened(FileHandle(0), FopenFlags::empty())
            }
            Some(_) => reply.error(Errno::EISDIR),
            None => reply.error(Errno::ENOENT),
        }
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
        let backend = self.backend.clone();
        self.runtime.spawn(async move {
            match backend.read(ino.0, offset, size).await {
                Ok(data) => reply.data(&data),
                Err(e) => reply.error(Self::failed(&backend, &e)),
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
        let backend = self.backend.clone();
        self.runtime.spawn(async move {
            let entries = match backend.list(ino.0).await {
                Ok(entries) => entries,
                Err(e) => return reply.error(Self::failed(&backend, &e)),
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
        let backend = self.backend.clone();
        self.runtime.spawn(async move {
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

/// `O_WRONLY | O_RDWR`: opening for writing is refused while read-only.
const fn libc_access_mode() -> i32 {
    0o1 | 0o2
}

/// A mounted filesystem; dropping it unmounts.
pub struct Mount {
    _session: fuser::BackgroundSession,
}

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
        MountOption::RO,
        MountOption::FSName("nextgdrive".into()),
        MountOption::Subtype("nextgdrive".into()),
        MountOption::DefaultPermissions,
        MountOption::NoSuid,
        MountOption::NoDev,
    ];
    let session = fuser::spawn_mount(fs, mountpoint, &config)?;
    Ok(Mount { _session: session })
}

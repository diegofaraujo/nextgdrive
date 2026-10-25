//! Mounting through an explicit `fusermount3` helper (`FUSERMOUNT_PATH`), as
//! a sandbox must: a Flatpak has no `/dev/fuse`, and fuser only falls back to
//! fusermount when opening it is denied, not when it is missing.
//!
//! The helper's protocol: it gets a socket in `_FUSE_COMMFD`, mounts, and
//! sends the `/dev/fuse` descriptor back over it (`SCM_RIGHTS`).

use std::ffi::OsStr;
use std::io::{self, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::process::{Command, Stdio};

use rustix::io::{Errno, FdFlags, fcntl_setfd};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SocketFlags, SocketType,
    recvmsg, socketpair,
};

/// Mounts at `mountpoint` with `options` (as for `fusermount3 -o`) and returns
/// the connection to hand to `fuser::Session::from_fd`.
pub(crate) fn mount(helper: &OsStr, mountpoint: &Path, options: &str) -> io::Result<OwnedFd> {
    // The helper's end must survive exec, so neither is created close-on-exec;
    // ours is made so right away (a child spawned meanwhile could keep the
    // other one open, which only delays noticing a failed mount).
    let (ours, theirs) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::empty(),
        None,
    )?;
    fcntl_setfd(&ours, FdFlags::CLOEXEC)?;
    let child = Command::new(helper)
        .arg("-o")
        .arg(options)
        .arg("--")
        .arg(mountpoint)
        .env("_FUSE_COMMFD", theirs.as_raw_fd().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // Only the helper holds its end now, so the read below ends if it exits.
    drop(theirs);
    let received = receive_fd(&ours);
    let output = child.wait_with_output()?;
    received.map_err(|e| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        match stderr.trim() {
            "" => e,
            message => io::Error::other(message.to_owned()),
        }
    })
}

/// Unmounts, even if busy (`-z`), as fuser does.
pub(crate) fn unmount(helper: &OsStr, mountpoint: &Path) {
    let _ = Command::new(helper)
        .args(["-u", "-q", "-z", "--"])
        .arg(mountpoint)
        .stdin(Stdio::null())
        .status();
}

fn receive_fd(socket: &OwnedFd) -> io::Result<OwnedFd> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let mut byte = [0u8];
    loop {
        match recvmsg(
            socket,
            &mut [IoSliceMut::new(&mut byte)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        ) {
            Err(Errno::INTR) => {}
            result => {
                result?;
                break;
            }
        }
    }
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(mut fds) = message
            && let Some(fd) = fds.next()
        {
            return Ok(fd);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "fusermount did not send the FUSE connection",
    ))
}

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Inspect,
    Stop,
    Cancel,
    DrawingRead,
    DrawingUndo,
    ReferenceRead,
    ReferenceCopy,
    PublicShareStart,
    PublicShareRead,
    PublicShareDeliver,
    PublicShareCancel,
}

pub fn socket_path(create: bool) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let dir = PathBuf::from(home).join(".config/apparent/agent-control");
    if create {
        std::fs::create_dir_all(&dir)?;
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() },
        "unsafe control directory"
    );
    if create {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    ensure!(
        std::fs::metadata(&dir)?.permissions().mode() & 0o077 == 0,
        "control directory is not private"
    );
    Ok(dir.join("capture.sock"))
}

pub fn same_user(stream: &UnixStream) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut _,
                &mut len,
            )
        };
        ensure!(
            rc == 0
                && len as usize == std::mem::size_of::<libc::ucred>()
                && cred.uid == unsafe { libc::geteuid() },
            "control peer is not this user"
        );
    }
    #[cfg(target_os = "macos")]
    {
        let mut uid = 0;
        let mut gid = 0;
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        ensure!(
            rc == 0 && uid == unsafe { libc::geteuid() },
            "control peer is not this user"
        );
    }
    Ok(())
}

pub fn clock_ms() -> u64 {
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    // CLOCK_MONOTONIC is available on both supported Unix targets.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
        0
    );
    time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn deadline_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "GUI capture control deadline expired",
    )
}

fn remaining_until(deadline: Instant) -> std::io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(deadline_error())
    } else {
        Ok(remaining)
    }
}

fn wait_ready(fd: libc::c_int, events: libc::c_short, deadline: Instant) -> std::io::Result<()> {
    loop {
        let remaining = remaining_until(deadline)?;
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let millis = remaining
            .as_millis()
            .saturating_add(1)
            .min(i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
        if result > 0 {
            remaining_until(deadline)?;
            return Ok(());
        }
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error());
        }
    }
}

/// Nonblocking from socket creation onward. Linux reports a full Unix accept queue as
/// EAGAIN (not EINPROGRESS); retry with a fresh socket without leaving a worker behind.
pub fn connect_until(path: &std::path::Path, deadline: Instant) -> std::io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid control socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dest, source) in address.sun_path.iter_mut().zip(bytes) {
        *dest = *source as libc::c_char;
    }
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as u8;
    }
    loop {
        remaining_until(deadline)?;
        #[cfg(target_os = "linux")]
        let flags = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::SOCK_STREAM;
        let fd = unsafe { libc::socket(libc::AF_UNIX, flags, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Own the descriptor immediately, including every error path below.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        #[cfg(target_os = "macos")]
        {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let enabled: libc::c_int = 1;
            if unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    (&enabled as *const libc::c_int).cast(),
                    std::mem::size_of_val(&enabled) as libc::socklen_t,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        stream.set_nonblocking(true)?;
        if unsafe { libc::connect(fd, (&address as *const libc::sockaddr_un).cast(), length) } == 0
        {
            return Ok(stream);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINPROGRESS) {
            wait_ready(fd, libc::POLLOUT, deadline)?;
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
            return Ok(stream);
        }
        if !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) {
            return Err(error);
        }
        drop(stream);
        std::thread::sleep(Duration::from_millis(2).min(remaining_until(deadline)?));
    }
}

/// One absolute deadline covers all partial reads/writes, not one timeout per byte.
pub struct DeadlineStream {
    pub stream: UnixStream,
    pub deadline: Instant,
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            wait_ready(self.stream.as_raw_fd(), libc::POLLIN, self.deadline)?;
            match self.stream.read(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                result => return result,
            }
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            wait_ready(self.stream.as_raw_fd(), libc::POLLOUT, self.deadline)?;
            match self.stream.write(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}


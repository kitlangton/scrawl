//! Existing media re-exec boundary shared with bounded local speech execution.
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::process::{Child, Command, Stdio};
type Result<T> = std::result::Result<T, &'static str>;
#[derive(Clone, Copy)]
pub(crate) enum Profile {
    Media,
    Speech,
}
impl Profile {
    fn memory(self) -> libc::rlim_t {
        match self {
            Self::Media => 1536 * 1024 * 1024,
            Self::Speech => 4 * 1024 * 1024 * 1024,
        }
    }
    // A sealed731MB model snapshot is a memfd and is subject to FSIZE too.
    fn file_size(self) -> libc::rlim_t {
        match self {
            Self::Media => 256 * 1024 * 1024,
            Self::Speech => 1024 * 1024 * 1024,
        }
    }
}
pub(crate) fn spawn_reexec(
    args: &[&str],
    input: &[u8],
    inherited_fds: &[(RawFd, RawFd)],
    profile: Profile,
    diagnostics: bool,
) -> Result<(Child, UnixStream)> {
    let (lifetime, monitor) = UnixStream::pair().map_err(|_| "internal")?;
    let copy = |fd| {
        let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        if fd < 0 {
            Err("internal")
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    };
    let mut descriptors = inherited_fds
        .iter()
        .map(|(from, to)| Ok((copy(*from)?, *to)))
        .collect::<Result<Vec<_>>>()?;
    descriptors.push((copy(monitor.as_raw_fd())?, 5));
    let parent_pid = std::process::id();
    let mut cmd = Command::new(std::env::current_exe().map_err(|_| "internal")?);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if diagnostics {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .env_clear();
    if diagnostics {
        cmd.env("APPARENT_MEDIA_TEST_DIAGNOSTICS", "1");
    }
    cmd.env("GST_REGISTRY", "/dev/null")
        .env("GST_REGISTRY_FORK", "no");
    // Preserve the launcher's native runtime closure in the re-exec.
    for key in [
        "LD_LIBRARY_PATH",
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
    ] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    unsafe {
        cmd.pre_exec(move || {
            #[cfg(target_os = "linux")]
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0
                || libc::getppid() as u32 != parent_pid
            {
                return Err(std::io::Error::other("parent unavailable"));
            }
            for (source, target) in &descriptors {
                if libc::dup2(source.as_raw_fd(), *target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            #[cfg(target_os = "linux")]
            {
                let memory = libc::rlimit {
                    rlim_cur: profile.memory(),
                    rlim_max: profile.memory(),
                };
                if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // Darwin rejects limits below its already-reserved virtual
            // mappings. Its decoder guards combined physical footprint.
            for (resource, limit) in [
                (libc::RLIMIT_CPU, 65),
                (libc::RLIMIT_FSIZE, profile.file_size()),
                (libc::RLIMIT_NOFILE, 128),
            ] {
                let lim = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|_| "internal")?;
    let write = child
        .stdin
        .take()
        .ok_or("internal")
        .and_then(|mut stdin| stdin.write_all(input).map_err(|_| "internal"));
    if let Err(error) = write {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok((child, lifetime))
}

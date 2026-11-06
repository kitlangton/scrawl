use std::env;
use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

const FD: RawFd = 3;
const MAX_TIMEOUT_MS: u64 = 60_000;

fn emit(state: &str) -> bool {
    let mut out = io::stdout().lock();
    writeln!(out, "{{\"protocol\":1,\"state\":\"{state}\"}}")
        .and_then(|_| out.flush())
        .is_ok()
}

fn errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn fstat(fd: RawFd) -> io::Result<libc::stat> {
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, st.as_mut_ptr()) } == 0 {
        Ok(unsafe { st.assume_init() })
    } else {
        Err(io::Error::last_os_error())
    }
}

fn publication_parent_metadata(st: &libc::stat) -> bool {
    (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
        && (st.st_uid == 0 || st.st_uid == unsafe { libc::geteuid() })
        && (st.st_mode & 0o022 == 0 || st.st_mode & libc::S_ISVTX as libc::mode_t != 0)
}

fn validate_fd(require_rw_regular: bool) -> Result<OwnedFd, &'static str> {
    validate_raw_fd(FD, require_rw_regular)
}

fn validate_raw_fd(fd: RawFd, require_rw_regular: bool) -> Result<OwnedFd, &'static str> {
    if fd < 0 {
        return Err("refused");
    }
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err("refused");
    }
    let st = fstat(fd).map_err(|_| "refused")?;
    if !no_allow_acl_fd(fd) || (require_rw_regular && st.st_uid != unsafe { libc::geteuid() }) {
        return Err("refused");
    }
    if require_rw_regular {
        if (st.st_mode & libc::S_IFMT) != libc::S_IFREG
            || st.st_mode & 0o077 != 0
            || st.st_mode & 0o200 == 0
            || st.st_nlink != 1
            || flags & libc::O_ACCMODE != libc::O_RDWR
        {
            return Err("refused");
        }
    } else if !publication_parent_metadata(&st) {
        return Err("refused");
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

enum Input {
    Eof,
    Release,
    Cancel,
    Bad,
}

fn poll_input(timeout_ms: i32) -> Result<Option<Input>, ()> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if result < 0 {
        if errno() == libc::EINTR {
            return Ok(None);
        }
        return Err(());
    }
    if result == 0 {
        return Ok(None);
    }
    if pfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(());
    }
    if pfd.revents & (libc::POLLIN | libc::POLLHUP) == 0 {
        return Err(());
    }
    let mut byte = 0u8;
    let n = unsafe {
        libc::read(
            libc::STDIN_FILENO,
            &mut byte as *mut u8 as *mut libc::c_void,
            1,
        )
    };
    if n == 0 {
        return Ok(Some(Input::Eof));
    }
    if n < 0 {
        if errno() == libc::EINTR || errno() == libc::EAGAIN {
            return Ok(None);
        }
        return Err(());
    }
    match byte {
        b'r' => Ok(Some(Input::Release)),
        b'c' => Ok(Some(Input::Cancel)),
        _ => Ok(Some(Input::Bad)),
    }
}

fn lock(timeout_ms: u64) -> &'static str {
    let fd = match validate_fd(true) {
        Ok(fd) => fd,
        Err(s) => return s,
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut attempted = false;
    loop {
        match poll_input(0) {
            Ok(Some(Input::Eof | Input::Release | Input::Cancel)) => return "cancelled",
            Ok(Some(Input::Bad)) | Err(()) => return "failed",
            _ => {}
        }
        if !attempted || Instant::now() < deadline {
            attempted = true;
            let rc = unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&fd),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            };
            if rc == 0 {
                match poll_input(0) {
                    Ok(Some(Input::Eof | Input::Release | Input::Cancel)) => {
                        unsafe {
                            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&fd), libc::LOCK_UN);
                        }
                        return "cancelled";
                    }
                    Ok(Some(Input::Bad)) | Err(()) => {
                        unsafe {
                            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&fd), libc::LOCK_UN);
                        }
                        return "failed";
                    }
                    _ => {}
                }
                break;
            }
            let e = errno();
            if e != libc::EWOULDBLOCK && e != libc::EAGAIN && e != libc::EINTR {
                return "failed";
            }
        } else {
            return "busy";
        }
        if Instant::now() >= deadline {
            return "busy";
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let ms = remaining.as_millis().min(10).max(1) as i32;
        match poll_input(ms) {
            Ok(Some(Input::Eof | Input::Release | Input::Cancel)) => return "cancelled",
            Ok(Some(Input::Bad)) | Err(()) => return "failed",
            _ => {}
        }
    }
    if !emit("acquired") {
        unsafe {
            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&fd), libc::LOCK_UN);
        }
        return "failed";
    }
    let final_state = loop {
        match poll_input(-1) {
            Ok(Some(Input::Eof | Input::Release | Input::Cancel)) => break "released",
            Ok(Some(Input::Bad)) | Err(()) => break "failed",
            Ok(None) => continue,
        }
    };
    if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&fd), libc::LOCK_UN) } != 0 {
        return "failed";
    }
    final_state
}

// Names are single UTF-8 POSIX components. Kernel-specific limits still apply.
fn basename(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4096
        && s != "."
        && s != ".."
        && !s.as_bytes().contains(&b'/')
        && !s.as_bytes().contains(&0)
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("validated basename")
}

fn publish(source: &str, destination: &str) -> &'static str {
    if !basename(source) || !basename(destination) || source == destination {
        return "refused";
    }
    let parent = match validate_fd(false) {
        Ok(fd) => fd,
        Err(s) => return s,
    };
    publish_at(
        std::os::fd::AsRawFd::as_raw_fd(&parent),
        source,
        destination,
        real_fsync,
    )
}

fn real_fsync(fd: RawFd) -> i32 {
    unsafe { libc::fsync(fd) }
}

fn private_directory(st: &libc::stat) -> bool {
    (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
        && st.st_uid == unsafe { libc::geteuid() }
        && st.st_mode & 0o077 == 0
}

fn parse_id(value: &str) -> Option<u64> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) || !value.bytes().all(|b| b.is_ascii_digit()) { return None; }
    value.parse::<u64>().ok()
}

fn same(st: &libc::stat, device: u64, inode: u64) -> bool {
    st.st_dev as u64 == device && st.st_ino as u64 == inode
}

fn private_regular(st: &libc::stat) -> bool {
    (st.st_mode & libc::S_IFMT) == libc::S_IFREG
        && st.st_uid == unsafe { libc::geteuid() }
        && st.st_mode & 0o7777 == 0o600
        && st.st_nlink == 1
        && st.st_size >= 0 && st.st_size <= 2_000_000
}
fn private_destination(st: &libc::stat) -> bool {
    (st.st_mode & libc::S_IFMT) == libc::S_IFREG
        && st.st_uid == unsafe { libc::geteuid() }
        && st.st_mode & 0o7777 & !0o600 == 0
        && st.st_size >= 0 && st.st_size <= 2_000_000
}
fn rename_file(parent: RawFd, source: *const libc::c_char, destination: *const libc::c_char, present: bool) -> Result<(), i32> {
    let rc = if present { unsafe { libc::renameat(parent, source, parent, destination) } }
        else { rename_noreplace(parent, source, destination) };
    if rc == 0 { Ok(()) } else { Err(errno()) }
}

fn replace_file(source: &str, destination: &str, source_device: &str, source_inode: &str,
    destination_present: bool, destination_device: &str, destination_inode: &str) -> &'static str {
    let parent = match validate_fd(false) { Ok(fd) => fd, Err(_) => return "refused" };
    replace_at(std::os::fd::AsRawFd::as_raw_fd(&parent), source, destination, source_device, source_inode, destination_present, destination_device, destination_inode, real_fsync, rename_file)
}

fn replace_at<F, R>(parent_fd: RawFd, source: &str, destination: &str, source_device: &str, source_inode: &str,
    destination_present: bool, destination_device: &str, destination_inode: &str, mut sync: F, mut rename: R) -> &'static str
where F: FnMut(RawFd) -> i32, R: FnMut(RawFd, *const libc::c_char, *const libc::c_char, bool) -> Result<(), i32> {
    if !basename(source) || !basename(destination) || source == destination { return "refused"; }
    let Some(sd) = parse_id(source_device) else { return "refused"; };
    let Some(si) = parse_id(source_inode) else { return "refused"; };
    let (dd, di) = if destination_present {
        let (Some(d), Some(i)) = (parse_id(destination_device), parse_id(destination_inode)) else { return "refused"; };
        (d, i)
    } else { (0, 0) };
    let pfd = parent_fd;
    let pst = match fstat(pfd) { Ok(s) => s, Err(_) => return "refused" };
    if !private_directory(&pst) || !no_allow_acl_fd(pfd) { return "refused"; }
    let src = cstr(source); let dst = cstr(destination);
    let raw = unsafe { libc::openat(pfd, src.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if raw < 0 { return "refused"; }
    let source_fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let sst = match fstat(raw) { Ok(s) => s, Err(_) => return "refused" };
    if !private_regular(&sst) || !no_allow_acl_fd(raw) { return "refused"; }
    if !same(&sst, sd, si) { return "destination_changed"; }
    if sync(raw) != 0 { return "refused"; }
    let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstatat(pfd, src.as_ptr(), current.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 { return "destination_changed"; }
    let current_source = unsafe { current.assume_init() };
    if !same(&current_source, sd, si) { return "destination_changed"; }
    if !private_regular(&current_source) || !no_allow_acl_fd(raw)
        || !matches!(fstat(pfd), Ok(ref st) if private_directory(st)) || !no_allow_acl_fd(pfd) { return "refused"; }
    let mut current_dst = std::mem::MaybeUninit::<libc::stat>::uninit();
    let dst_rc = unsafe { libc::fstatat(pfd, dst.as_ptr(), current_dst.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
    if destination_present {
        if dst_rc != 0 { return if errno() == libc::ENOENT { "destination_changed" } else { "refused" }; }
        let current_dst = unsafe { current_dst.assume_init() };
        if !private_destination(&current_dst) || !no_allow_acl_at(pfd, dst.as_ptr()) || !same(&current_dst, dd, di) { return "destination_changed"; }
    } else if dst_rc == 0 { return "destination_changed"; }
    else if errno() != libc::ENOENT { return "refused"; }
    if let Err(e) = rename(pfd, src.as_ptr(), dst.as_ptr(), destination_present) {
        return if !destination_present && (e == libc::EEXIST || e == libc::ENOTEMPTY) { "destination_changed" } else { "publication_uncertain" };
    }
    if sync(pfd) != 0 { return "publication_uncertain"; }
    let mut final_st = std::mem::MaybeUninit::<libc::stat>::uninit();
    let final_rc = unsafe { libc::fstatat(pfd, dst.as_ptr(), final_st.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
    let mut source_st = std::mem::MaybeUninit::<libc::stat>::uninit();
    let source_rc = unsafe { libc::fstatat(pfd, src.as_ptr(), source_st.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
    let source_error = errno();
    if final_rc != 0 || source_rc == 0 || source_error != libc::ENOENT { return "publication_uncertain"; }
    let final_st = unsafe { final_st.assume_init() };
    if !same(&final_st, sd, si) || !private_regular(&final_st) || !no_allow_acl_at(pfd, dst.as_ptr())
        || !matches!(fstat(pfd), Ok(ref st) if private_directory(st)) || !no_allow_acl_fd(pfd) { return "publication_uncertain"; }
    drop(source_fd); "replaced"
}

#[cfg(target_os = "macos")]
fn no_allow_acl_at(dir: RawFd, name: *const libc::c_char) -> bool {
    let fd = unsafe { libc::openat(dir, name, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 { return false; }
    let ok = no_allow_acl_fd(fd); unsafe { libc::close(fd); } ok
}
#[cfg(target_os = "linux")]
fn no_allow_acl_at(_dir: RawFd, _name: *const libc::c_char) -> bool { true }

fn publish_at<F>(parent_fd: RawFd, source: &str, destination: &str, mut sync: F) -> &'static str
where
    F: FnMut(RawFd) -> i32,
{
    if !basename(source) || !basename(destination) || source == destination {
        return "refused";
    }
    let pst = match fstat(parent_fd) {
        Ok(s) => s,
        Err(_) => return "refused",
    };
    if !publication_parent_metadata(&pst) || !no_allow_acl_fd(parent_fd) {
        return "refused";
    }
    let src = cstr(source);
    let dst = cstr(destination);
    let raw = unsafe {
        libc::openat(
            parent_fd,
            src.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return "refused";
    }
    let source_fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let sst = match fstat(raw) {
        Ok(s) => s,
        Err(_) => return "refused",
    };
    if sst.st_uid != unsafe { libc::geteuid() } || sst.st_mode & 0o077 != 0 || !no_allow_acl_fd(raw)
    {
        return "refused";
    }
    if sync(raw) != 0 {
        return "refused";
    }
    let rc = rename_noreplace(parent_fd, src.as_ptr(), dst.as_ptr());
    if rc != 0 {
        let e = errno();
        return if e == libc::EEXIST || e == libc::ENOTEMPTY {
            "destination_exists"
        } else {
            "refused"
        };
    }
    if sync(parent_fd) != 0 {
        return "publication_uncertain";
    }
    let mut dst_st = std::mem::MaybeUninit::<libc::stat>::uninit();
    let dst_ok = unsafe {
        libc::fstatat(
            parent_fd,
            dst.as_ptr(),
            dst_st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0;
    let mut src_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let src_st = unsafe {
        libc::fstatat(
            parent_fd,
            src.as_ptr(),
            src_stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if !dst_ok || src_st == 0 || errno() != libc::ENOENT {
        return "publication_uncertain";
    }
    let dst_st = unsafe { dst_st.assume_init() };
    if dst_st.st_dev != sst.st_dev || dst_st.st_ino != sst.st_ino {
        return "publication_uncertain";
    }
    drop(source_fd);
    "published"
}

#[cfg(target_os = "linux")]
fn rename_noreplace(dir: RawFd, src: *const libc::c_char, dst: *const libc::c_char) -> i32 {
    unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            dir,
            src,
            dir,
            dst,
            libc::RENAME_NOREPLACE,
        ) as i32
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(dir: RawFd, src: *const libc::c_char, dst: *const libc::c_char) -> i32 {
    unsafe { libc::renameatx_np(dir, src, dir, dst, libc::RENAME_EXCL) }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("native resources support Linux and macOS only");

// Adapted from Connect's no_allow_acl: Darwin ACL grants can override mode bits.
// Only OS ACL facts are examined; caller owns pathname and file/dir policy.
#[cfg(target_os = "macos")]
fn no_allow_acl_fd(fd: RawFd) -> bool {
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut libc::c_void;
        fn acl_get_entry(
            acl: *mut libc::c_void,
            selector: libc::c_int,
            entry: *mut *mut libc::c_void,
        ) -> libc::c_int;
        fn acl_get_tag_type(entry: *mut libc::c_void, tag: *mut libc::c_int) -> libc::c_int;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }
    let acl = unsafe { acl_get_fd_np(fd, 0x100) };
    if acl.is_null() {
        return errno() == libc::ENOENT;
    }
    let mut entry = std::ptr::null_mut();
    let mut selector = 0;
    let permitted = loop {
        if unsafe { acl_get_entry(acl, selector, &mut entry) } != 0 {
            break errno() == libc::EINVAL;
        }
        let mut tag = 0;
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 || tag != 2 {
            break false;
        }
        selector = -1;
    };
    unsafe {
        acl_free(acl);
    }
    permitted
}
#[cfg(target_os = "linux")]
fn no_allow_acl_fd(_fd: RawFd) -> bool {
    // POSIX ACL effective access is bounded by the mode's ACL mask; callers
    // must still check ownership, private mode and ancestor metadata.
    true
}

fn args() -> Result<(String, Vec<String>), &'static str> {
    let mut it = env::args_os().skip(1);
    let command = it
        .next()
        .ok_or("refused")?
        .into_string()
        .map_err(|_| "refused")?;
    let rest = it
        .map(|value| value.into_string().map_err(|_| "refused"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((command, rest))
}

fn run() -> &'static str {
    let (cmd, args) = match args() {
        Ok(v) => v,
        Err(s) => return s,
    };
    match cmd.as_str() {
        "acl" if args.is_empty() => match fstat(FD) {
            Ok(st)
                if (st.st_uid == 0 || st.st_uid == unsafe { libc::geteuid() })
                    && matches!(st.st_mode & libc::S_IFMT, libc::S_IFREG | libc::S_IFDIR)
                    && no_allow_acl_fd(FD) =>
            {
                "accepted"
            }
            _ => "refused",
        },
        "lock"
            if args.len() == 2
                && args[0] == "--timeout-ms"
                && !args[1].is_empty()
                && args[1].bytes().all(|b| b.is_ascii_digit()) =>
        {
            match args[1].parse::<u64>() {
                Ok(n) if n <= MAX_TIMEOUT_MS => lock(n),
                _ => "refused",
            }
        }
        "publish" if args.len() == 4 && args[0] == "--source" && args[2] == "--destination" => {
            publish(&args[1], &args[3])
        }
        "replace" if args.len() == 14 && args[0] == "--source" && args[2] == "--destination"
            && args[4] == "--source-device" && args[6] == "--source-inode"
            && args[8] == "--destination-present" && args[10] == "--destination-device"
            && args[12] == "--destination-inode" => {
            match args[9].as_str() {
                "0" => replace_file(&args[1], &args[3], &args[5], &args[7], false, &args[11], &args[13]),
                "1" => replace_file(&args[1], &args[3], &args[5], &args[7], true, &args[11], &args[13]),
                _ => "refused",
            }
        }
        _ => "refused",
    }
}

fn main() {
    let result = run();
    let _ = emit(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn temp() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "apparent-resource-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn protocol_names_and_metadata_guards() {
        assert!(basename("a-_9.txt"));
        for valid in ["a b", "résumé.apparent", "a\n"] {
            assert!(basename(valid));
        }
        for bad in ["", ".", "..", "/tmp/x", "a\0", &"x".repeat(4097)] {
            assert!(!basename(bad));
        }
        let dir = temp();
        let file = dir.join("lock");
        fs::write(&file, b"").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let metadata = fs::metadata(&file).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.nlink(), 1);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        assert_ne!(fs::metadata(&file).unwrap().permissions().mode() & 0o077, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn valid_private_lock_fd_is_accepted_and_group_mode_or_hardlink_is_refused() {
        let dir = temp();
        let path = dir.join("lock");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let raw = unsafe {
            libc::open(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        let owned = validate_raw_fd(raw, true).expect("private read-write lock accepted");
        drop(owned);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let raw = unsafe {
            libc::open(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        assert!(validate_raw_fd(raw, true).is_err());
        unsafe {
            libc::close(raw);
        }

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&path, dir.join("lock-alias")).unwrap();
        let raw = unsafe {
            libc::open(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        assert!(validate_raw_fd(raw, true).is_err());
        unsafe {
            libc::close(raw);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lock_fd_rejects_readonly_wrong_type_and_symlink_descriptors() {
        let dir = temp();
        let path = dir.join("lock");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let ro = unsafe {
            libc::open(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        assert!(validate_raw_fd(ro, true).is_err());
        unsafe {
            libc::close(ro);
        }

        let wrong = fs::File::open(&dir).unwrap();
        assert!(validate_raw_fd(wrong.as_raw_fd(), true).is_err());
        #[cfg(target_os = "linux")]
        {
            symlink("lock", dir.join("link")).unwrap();
            let link = unsafe {
                libc::open(
                    std::ffi::CString::new(dir.join("link").to_str().unwrap())
                        .unwrap()
                        .as_ptr(),
                    libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            assert!(validate_raw_fd(link, true).is_err());
            unsafe {
                libc::close(link);
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn publication_no_follow_private_source_and_no_replace() {
        let dir = temp();
        let parent = fs::File::open(&dir).unwrap();
        fs::create_dir(dir.join("stage")).unwrap();
        fs::set_permissions(dir.join("stage"), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            publish_at(parent.as_raw_fd(), "stage", "final", real_fsync),
            "published"
        );
        assert!(dir.join("final").is_dir());

        fs::create_dir(dir.join("stage2")).unwrap();
        fs::set_permissions(dir.join("stage2"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(dir.join("exists")).unwrap();
        assert_eq!(
            publish_at(parent.as_raw_fd(), "stage2", "exists", real_fsync),
            "destination_exists"
        );
        assert!(dir.join("stage2").exists());
        assert!(dir.join("exists").is_dir());

        fs::create_dir(dir.join("loose")).unwrap();
        fs::set_permissions(dir.join("loose"), fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(
            publish_at(parent.as_raw_fd(), "loose", "loose-dst", real_fsync),
            "refused"
        );
        symlink("final", dir.join("link")).unwrap();
        assert_eq!(
            publish_at(parent.as_raw_fd(), "link", "link-dst", real_fsync),
            "refused"
        );
        fs::write(dir.join("plain"), b"not a directory").unwrap();
        assert_eq!(
            publish_at(parent.as_raw_fd(), "plain", "plain-dst", real_fsync),
            "refused"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parent_fsync_failure_after_rename_is_uncertain_and_preserves_destination() {
        let dir = temp();
        let parent = fs::File::open(&dir).unwrap();
        fs::create_dir(dir.join("stage")).unwrap();
        fs::set_permissions(dir.join("stage"), fs::Permissions::from_mode(0o700)).unwrap();
        let stage_meta = fs::metadata(dir.join("stage")).unwrap();
        let mut sync_calls = 0;
        let result = publish_at(parent.as_raw_fd(), "stage", "final", |fd| {
            sync_calls += 1;
            if sync_calls == 1 {
                real_fsync(fd)
            } else {
                -1
            }
        });

        assert_eq!(result, "publication_uncertain");
        assert_eq!(
            sync_calls, 2,
            "source fsync completed before parent fsync failure"
        );
        assert!(!dir.join("stage").exists());
        let final_meta = fs::metadata(dir.join("final")).unwrap();
        assert_eq!(final_meta.dev(), stage_meta.dev());
        assert_eq!(final_meta.ino(), stage_meta.ino());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_publish_names_refuse_without_mutation() {
        let dir = temp();
        let parent = fs::File::open(&dir).unwrap();
        fs::create_dir(dir.join("stage")).unwrap();
        assert_eq!(
            publish_at(parent.as_raw_fd(), "../stage", "dst", real_fsync),
            "refused"
        );
        assert_eq!(
            publish_at(parent.as_raw_fd(), "stage", "stage", real_fsync),
            "refused"
        );
        assert!(dir.join("stage").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replacement_rechecks_private_metadata_after_source_sync() {
        for changed in ["source_mode", "source_links", "parent_mode"] {
            let dir = temp();
            let parent = fs::File::open(&dir).unwrap();
            fs::write(dir.join("stage"), b"new").unwrap();
            fs::write(dir.join("final"), b"old").unwrap();
            for name in ["stage", "final"] { fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o600)).unwrap(); }
            let src = fs::metadata(dir.join("stage")).unwrap();
            let dst = fs::metadata(dir.join("final")).unwrap();
            let mut calls = 0;
            let state = replace_at(parent.as_raw_fd(), "stage", "final", &src.dev().to_string(), &src.ino().to_string(), true,
                &dst.dev().to_string(), &dst.ino().to_string(), |fd| {
                    calls += 1;
                    if calls == 1 {
                        match changed {
                            "source_mode" => fs::set_permissions(dir.join("stage"), fs::Permissions::from_mode(0o644)).unwrap(),
                            "source_links" => fs::hard_link(dir.join("stage"), dir.join("alias")).unwrap(),
                            _ => fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap(),
                        }
                    }
                    real_fsync(fd)
                }, rename_file);
            let source_present = dir.join("stage").exists();
            let destination = fs::read(dir.join("final")).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
            fs::remove_dir_all(dir).unwrap();
            assert_eq!(state, "refused", "changed={changed}");
            assert!(source_present, "changed={changed}");
            assert_eq!(destination, b"old", "changed={changed}");
        }
    }

    #[test]
    fn replacement_parent_sync_failure_is_uncertain_and_keeps_renamed_file() {
        let dir = temp();
        let parent = fs::File::open(&dir).unwrap();
        fs::write(dir.join("stage"), b"replacement").unwrap();
        fs::set_permissions(dir.join("stage"), fs::Permissions::from_mode(0o600)).unwrap();
        let source = fs::metadata(dir.join("stage")).unwrap();
        let mut calls = 0;
        let state = replace_at(parent.as_raw_fd(), "stage", "final", &source.dev().to_string(), &source.ino().to_string(), false, "0", "0", |fd| {
            calls += 1;
            if calls == 1 { real_fsync(fd) } else { -1 }
        }, rename_file);
        assert_eq!(state, "publication_uncertain");
        assert_eq!(calls, 2);
        assert!(!dir.join("stage").exists());
        assert_eq!(fs::read(dir.join("final")).unwrap(), b"replacement");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replacement_lost_rename_acknowledgement_keeps_effect_and_reports_uncertainty() {
        let dir = temp();
        let parent = fs::File::open(&dir).unwrap();
        fs::write(dir.join("stage"), b"new bytes").unwrap();
        fs::set_permissions(dir.join("stage"), fs::Permissions::from_mode(0o600)).unwrap();
        let source = fs::metadata(dir.join("stage")).unwrap();
        let state = replace_at(parent.as_raw_fd(), "stage", "final", &source.dev().to_string(), &source.ino().to_string(),
            false, "0", "0", real_fsync, |fd, source, destination, present| {
                rename_file(fd, source, destination, present)?;
                Err(libc::EIO) // Effect succeeded; its acknowledgement was lost.
            });
        assert_eq!(state, "publication_uncertain");
        assert!(!dir.join("stage").exists());
        assert_eq!(fs::read(dir.join("final")).unwrap(), b"new bytes");
        fs::remove_dir_all(dir).unwrap();
    }
}

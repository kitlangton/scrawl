//! The Windows shell: opening and revealing files, the Recycle Bin, version info, NTFS change
//! times, thread priority and random bytes.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileBasicInfo,
    GetFileInformationByHandleEx, GetFileVersionInfoSizeW, GetFileVersionInfoW, OPEN_EXISTING,
    VerQueryValueW,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
};
use windows::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    SHFileOperationW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

use super::wide;

/// The shell, unlike the file system, rejects `/` in a path (`…\scrawl/captures\…`), so an
/// absolute path is rebuilt from its components with `\` between them. URLs pass through.
fn shell_path(target: &Path) -> std::path::PathBuf {
    if target.is_absolute() {
        target.components().collect()
    } else {
        target.to_owned()
    }
}

/// Opens a file, folder or URL with its default handler.
pub fn open(target: &Path) -> Result<()> {
    let target = wide(&shell_path(target).to_string_lossy());
    let verb = wide("open");
    // ShellExecute pumps messages while it starts the handler. On GPUI's thread that runs queued
    // tasks while the calling event still holds the app, and their panic aborts Scrawl.
    let result = std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        }
        .0 as isize
    })
    .join()
    .unwrap_or(0);
    // ShellExecute returns a value above 32 on success.
    if result <= 32 {
        bail!("Could not open it (ShellExecute error {result})");
    }
    Ok(())
}

/// Shows a file selected in a File Explorer window.
pub fn reveal(target: &Path) -> Result<()> {
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(target.as_os_str());
    std::process::Command::new("explorer.exe")
        .arg(arg)
        .spawn()
        .context("Could not start File Explorer")?;
    Ok(())
}

/// Moves a file or folder to the Recycle Bin.
pub fn recycle(path: &Path) -> Result<()> {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    // A double-NUL-terminated list of one path.
    let mut from: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        bail!(
            "Could not move {} to the Recycle Bin (error {code})",
            path.display()
        );
    }
    Ok(())
}

/// The `FileDescription` in an executable's version info ("Google Chrome").
pub fn file_description(path: &Path) -> Option<String> {
    let name = wide(&path.to_string_lossy());
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(name.as_ptr()), None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0_u8; size as usize];
    unsafe { GetFileVersionInfoW(PCWSTR(name.as_ptr()), None, size, data.as_mut_ptr().cast()) }
        .ok()?;
    // The first language and code page in the translation table.
    let mut translation: *mut u16 = std::ptr::null_mut();
    let mut len = 0_u32;
    let query = wide(r"\VarFileInfo\Translation");
    let found = unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(query.as_ptr()),
            (&raw mut translation).cast(),
            &mut len,
        )
    };
    let (lang, page) = if found.as_bool() && len >= 4 && !translation.is_null() {
        unsafe { (*translation, *translation.add(1)) }
    } else {
        (0x0409, 0x04B0)
    };
    let key = wide(&format!(
        r"\StringFileInfo\{lang:04x}{page:04x}\FileDescription"
    ));
    let mut value: *mut u16 = std::ptr::null_mut();
    let mut chars = 0_u32;
    let found = unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(key.as_ptr()),
            (&raw mut value).cast(),
            &mut chars,
        )
    };
    if !found.as_bool() || value.is_null() || chars == 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(value, chars as usize) };
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    let text = String::from_utf16_lossy(&slice[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// When an entry in a folder was last added, removed or renamed (NTFS `ChangeTime`).
pub fn change_time(path: &Path) -> Option<SystemTime> {
    let name = wide(&path.to_string_lossy());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .ok()?;
    let handle = Handle(handle);
    let mut info = FILE_BASIC_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle.0,
            FileBasicInfo,
            (&raw mut info).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    }
    .ok()?;
    // 100 ns ticks since 1601-01-01.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let ticks = info.ChangeTime.checked_sub(UNIX_EPOCH_TICKS)?;
    Some(UNIX_EPOCH + Duration::from_nanos(u64::try_from(ticks).ok()? * 100))
}

/// Closes the console window Windows opened for this process when it was launched from
/// Explorer or the Start menu (`scrawl.exe` is a console program so the CLI works). A console
/// shared with a terminal it was started from stays, so its log keeps printing there.
pub fn release_own_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut processes = [0_u32; 2];
    let count = unsafe { GetConsoleProcessList(&mut processes) };
    if count == 1 {
        let _ = unsafe { FreeConsole() };
    }
}

/// Set on a copy started by `onboarding::relaunch`: the process id of the copy that started it.
pub const RELAUNCHED_BY: &str = "SCRAWL_RELAUNCHED_BY";

/// A copy started by `onboarding::relaunch` waits (at most 10 s) for the one that started it to
/// quit, so there are never two running.
pub fn wait_for_relauncher() {
    if let Some(pid) = std::env::var(RELAUNCHED_BY)
        .ok()
        .and_then(|v| v.parse().ok())
    {
        wait_for_exit(pid, Duration::from_secs(10));
    }
}

/// Waits for process `pid` to exit, at most `limit`. Returns whether it has (or never ran).
fn wait_for_exit(pid: u32, limit: Duration) -> bool {
    use windows::Win32::Foundation::WAIT_TIMEOUT;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return true;
    };
    let waited = unsafe { WaitForSingleObject(process, limit.as_millis() as u32) };
    let _ = unsafe { CloseHandle(process) };
    waited != WAIT_TIMEOUT
}

/// Windows 11 (build 22000 and later), which has Segoe Fluent Icons.
pub fn windows_11() -> bool {
    static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *BUILD.get_or_init(|| {
        let mut info = windows::Win32::System::SystemInformation::OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<
                windows::Win32::System::SystemInformation::OSVERSIONINFOW,
            >() as u32,
            ..Default::default()
        };
        // RtlGetVersion isn't subject to the compatibility shims GetVersionEx is.
        let status = unsafe { windows::Wdk::System::SystemServices::RtlGetVersion(&mut info) };
        if status.is_ok() {
            info.dwBuildNumber
        } else {
            0
        }
    }) >= 22000
}

/// Puts the calling thread in background mode (low CPU and I/O priority).
pub fn background_thread() {
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
}

pub fn random_bytes(bytes: &mut [u8]) -> Result<()> {
    unsafe { BCryptGenRandom(None, bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .context("Could not read random bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relaunched_copy_waits_for_the_one_that_started_it() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 2 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        assert!(!wait_for_exit(child.id(), Duration::from_millis(50)));
        assert!(wait_for_exit(child.id(), Duration::from_secs(10)));
        let _ = child.wait();
        // Already gone (or never there).
        assert!(wait_for_exit(u32::MAX - 3, Duration::from_millis(50)));
    }

    #[test]
    fn absolute_paths_lose_their_forward_slashes_and_urls_stay() {
        let mixed = Path::new(r"C:\Users\a\AppData\Local\scrawl/captures\one.scrawl");
        assert_eq!(
            shell_path(mixed).to_string_lossy(),
            r"C:\Users\a\AppData\Local\scrawl\captures\one.scrawl"
        );
        let url = Path::new("https://github.com/kitlangton/scrawl/releases");
        assert_eq!(shell_path(url), url);
    }
}

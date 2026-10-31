//! Launch at login: a value under the user's `Run` key, as Settings › Apps › Startup lists it.
//! That page (and Task Manager) can turn it off without removing it, in `StartupApproved`.

use anyhow::{Result, bail};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::PCWSTR;

use super::wide;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "Scrawl";

/// The command Windows runs at login: this executable's tray app.
fn command() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" app", exe.display()))
}

fn registered() -> bool {
    let (key, value) = (wide(RUN), wide(NAME));
    let mut size = 0_u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    }
    .is_ok()
}

/// Whether Startup apps has the entry switched off (its first byte odd past 2: 3 is off).
fn switched_off() -> bool {
    let (key, value) = (wide(APPROVED), wide(NAME));
    let mut data = [0_u8; 12];
    let mut size = data.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && size > 0 && data[0] & 1 == 1
}

/// (registered, switched off in Startup apps).
pub fn status() -> (bool, bool) {
    let on = registered();
    (on, on && switched_off())
}

pub fn set(on: bool) -> Result<()> {
    let (key, value) = (wide(RUN), wide(NAME));
    let status = if on {
        let data = wide(&command()?);
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(value.as_ptr()),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        }
    } else {
        unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(value.as_ptr()),
            )
        }
    };
    if status.is_err() && on {
        bail!("Could not write the Run key: {status:?}");
    }
    Ok(())
}

/// Settings › Apps › Startup, where an entry switched off goes back on.
pub fn open_startup_settings() {
    let _ = super::shell::open(std::path::Path::new("ms-settings:startupapps"));
}

#[cfg(test)]
mod build_version { include!("enablement/build_version.rs"); }
mod capture;
mod composition;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod connect;
mod shared;

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut args = std::env::args_os();
        let _program = args.next();
        if args.next().as_deref()
            == Some(std::ffi::OsStr::new("--internal-wayland-clipboard-owner"))
            && args.next().is_none()
        {
            return capture::desktop::linux::clipboard::run_owner_protocol();
        }
    }
    composition::cli::run()
}

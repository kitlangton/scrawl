//! `scrawl` on PATH, on Windows: Settings › Agents writes a `scrawl.cmd` shim that runs this
//! executable into `%LOCALAPPDATA%\Microsoft\WindowsApps`, which is on every user's PATH. A shim
//! rather than a link: symbolic links need Developer Mode, and a hard link can't cross drives.
//! Same API as the macOS module (`command_line.rs`), so Settings is the same on both.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const NAME: &str = "scrawl.cmd";

/// The executable the shim runs and the folders to look in.
#[derive(Clone, Debug)]
pub struct Places {
    /// This executable.
    pub target: Option<PathBuf>,
    /// Where a new shim may go, in order. The first that exists and is writable is used; the
    /// last is created if needed.
    pub install: Vec<PathBuf>,
    /// Also checked for an existing shim.
    pub also: Vec<PathBuf>,
}

impl Places {
    pub fn system() -> Self {
        let target = std::env::current_exe().ok();
        let install = std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join(r"Microsoft\WindowsApps"))
            .into_iter()
            .collect();
        Self {
            target,
            install,
            also: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// No executable to run (never on Windows; kept for the shared Settings row).
    Unavailable,
    /// `link` runs this Scrawl. `homebrew` is always false on Windows.
    Installed { link: PathBuf, homebrew: bool },
    /// Install would write `link`.
    Missing { link: PathBuf },
    /// `link` runs another copy of Scrawl or one that's gone; Install replaces it.
    Stale { link: PathBuf },
    /// Something else is called `scrawl` there, and Scrawl leaves it alone.
    Taken { link: PathBuf },
    /// No folder Scrawl may write to.
    Nowhere,
}

/// The shim's text: run `target` with every argument.
fn shim(target: &Path) -> String {
    format!("@\"{}\" %*\r\n", target.display())
}

/// The executable a Scrawl shim runs, if `link` is one.
fn shim_target(link: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(link).ok()?;
    let rest = text.trim().strip_prefix("@\"")?;
    let (target, args) = rest.split_once('"')?;
    (args.trim() == "%*" && target.to_ascii_lowercase().ends_with("scrawl.exe"))
        .then(|| PathBuf::from(target))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".scrawl-write-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    dir.is_dir() && ok
}

pub fn state(places: &Places) -> State {
    let Some(target) = &places.target else {
        return State::Unavailable;
    };
    if let Some(link) = places
        .install
        .iter()
        .chain(&places.also)
        .map(|dir| dir.join(NAME))
        .find(|link| shim_target(link).is_some_and(|to| same_file(&to, target)))
    {
        return State::Installed {
            link,
            homebrew: false,
        };
    }
    let last = places.install.len().saturating_sub(1);
    let Some(dir) = places
        .install
        .iter()
        .enumerate()
        .find(|(i, dir)| writable(dir) || *i == last && !dir.exists())
        .map(|(_, dir)| dir)
    else {
        return State::Nowhere;
    };
    let link = dir.join(NAME);
    let exe = dir.join("scrawl.exe");
    if exe.exists() {
        State::Taken { link: exe }
    } else if !link.exists() {
        State::Missing { link }
    } else if shim_target(&link).is_some() {
        State::Stale { link }
    } else {
        State::Taken { link }
    }
}

/// Writes the shim and returns where. One that's already right is left alone.
pub fn install(places: &Places) -> Result<PathBuf> {
    let link = match state(places) {
        State::Installed { link, .. } => return Ok(link),
        State::Missing { link } | State::Stale { link } => link,
        State::Taken { link } => bail!("{} is another program", tilde(&link)),
        State::Unavailable => bail!("there's no Scrawl executable to run"),
        State::Nowhere => bail!("there's no folder Scrawl can write to"),
    };
    let target = places.target.as_ref().context("no Scrawl executable")?;
    let dir = link.parent().context("no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", tilde(dir)))?;
    std::fs::write(&link, shim(target))
        .with_context(|| format!("couldn't write to {}", tilde(dir)))?;
    Ok(link)
}

/// Removes the shim Install wrote (or a stale one). Anything else stays.
pub fn uninstall(places: &Places) -> Result<()> {
    let link = match state(places) {
        State::Installed { link, .. } | State::Stale { link } => link,
        _ => return Ok(()),
    };
    if shim_target(&link).is_some() {
        std::fs::remove_file(&link).with_context(|| format!("couldn't remove {}", tilde(&link)))?;
    }
    Ok(())
}

/// `~\AppData\Local\…` rather than `C:\Users\kit\AppData\Local\…`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

/// Whether a shim here is likely on the user's PATH already: WindowsApps is on every user's
/// PATH, and any folder that's on this process's PATH counts too.
pub fn usually_on_path(link: &Path) -> bool {
    let Some(dir) = link.parent() else {
        return false;
    };
    let windows_apps = dir.ends_with(r"Microsoft\WindowsApps");
    windows_apps
        || std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|p| same_file(&p, dir)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "scrawl-cli-{name}-{}-{}",
                std::process::id(),
                crate::pipeline::now_epoch_ms()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn places(dir: &TempDir, target: &Path) -> Places {
        Places {
            target: Some(target.to_path_buf()),
            install: vec![dir.0.join("bin")],
            also: Vec::new(),
        }
    }

    #[test]
    fn a_shim_runs_this_scrawl_and_uninstall_removes_it() {
        let dir = TempDir::new("shim");
        let exe = dir.0.join("app").join("scrawl.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let places = places(&dir, &exe);
        let link = dir.0.join("bin").join(NAME);
        assert_eq!(state(&places), State::Missing { link: link.clone() });
        assert_eq!(install(&places).unwrap(), link);
        assert_eq!(shim_target(&link).as_deref(), Some(exe.as_path()));
        assert_eq!(
            state(&places),
            State::Installed {
                link: link.clone(),
                homebrew: false
            }
        );
        // Another copy's shim is stale and replaced.
        let other = dir.0.join("old").join("scrawl.exe");
        std::fs::write(&link, shim(&other)).unwrap();
        assert_eq!(state(&places), State::Stale { link: link.clone() });
        install(&places).unwrap();
        uninstall(&places).unwrap();
        assert!(!link.exists());
    }

    #[test]
    fn another_program_called_scrawl_is_left_alone() {
        let dir = TempDir::new("taken");
        let exe = dir.0.join("scrawl.exe");
        std::fs::write(&exe, "").unwrap();
        let places = places(&dir, &exe);
        let bin = dir.0.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(NAME), "@echo other\r\n").unwrap();
        assert_eq!(
            state(&places),
            State::Taken {
                link: bin.join(NAME)
            }
        );
        assert!(install(&places).is_err());
        uninstall(&places).unwrap();
        assert_eq!(
            std::fs::read_to_string(bin.join(NAME)).unwrap(),
            "@echo other\r\n"
        );
    }
}

//! `scrawl` on PATH. Settings › Agents links the binary inside Scrawl.app into a folder on PATH,
//! so agents and terminals run the app's own copy and it updates with the app. The binary finds
//! `scrawl-helper` through its real path (`helper::path`), so it works through the link.
//! Homebrew's cask (`binary "#{appdir}/Scrawl.app/Contents/MacOS/scrawl"`) makes the same link,
//! and `brew` accepts one Scrawl made because it points at the same file.
//!
//! Windows has no Scrawl.app (`State::Unavailable`); `scrawl.exe` is a console program already.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const NAME: &str = "scrawl";

/// The binary to link and the folders to look in.
#[derive(Clone, Debug)]
pub struct Places {
    /// This executable's real path, when it is inside Scrawl.app.
    pub target: Option<PathBuf>,
    /// Where a new link may go, in order. The first that exists and is writable is used; the
    /// last is created if needed.
    pub install: Vec<PathBuf>,
    /// Also checked for an existing link (`~/bin`, where `install-app.sh` puts one).
    pub also: Vec<PathBuf>,
}

impl Places {
    pub fn system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        // On Linux the AppImage is the app (`$APPIMAGE` names it while it runs).
        #[cfg(target_os = "linux")]
        if let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
            return Self {
                target: std::fs::canonicalize(&appimage).ok(),
                install: home.iter().map(|h| h.join(".local/bin")).collect(),
                also: home.map(|h| h.join("bin")).into_iter().collect(),
            };
        }
        let target = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .ok()
            .filter(|exe| {
                exe.ancestors()
                    .any(|p| p.extension().is_some_and(|e| e == "app"))
            });
        let mut install = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        install.extend(home.as_ref().map(|h| h.join(".local/bin")));
        Self {
            target,
            install,
            also: home.map(|h| h.join("bin")).into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not running from Scrawl.app (a development build).
    Unavailable,
    /// `link` points at this Scrawl. `homebrew` when the cask made it.
    Installed { link: PathBuf, homebrew: bool },
    /// Install would make `link`.
    Missing { link: PathBuf },
    /// `link` points at another copy of Scrawl or at nothing; Install replaces it.
    Stale { link: PathBuf },
    /// Something else is called `scrawl` there, and Scrawl leaves it alone.
    Taken { link: PathBuf },
    /// No folder Scrawl may write to.
    Nowhere,
}

#[cfg(not(unix))]
fn writable(_dir: &Path) -> bool {
    false
}

#[cfg(unix)]
fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    dir.is_dir() && unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0
}

/// A symlink to some copy of Scrawl.app's binary, whether or not it still exists.
fn scrawl_link(link: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|to| to.ends_with("Contents/MacOS/scrawl"))
}

fn links_to(link: &Path, target: &Path) -> bool {
    link.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::canonicalize(link).is_ok_and(|real| real == target)
}

/// Homebrew keeps a cask's files under `<prefix>/Caskroom/<name>`, beside `<prefix>/bin`.
fn from_homebrew(link: &Path) -> bool {
    link.parent()
        .and_then(Path::parent)
        .is_some_and(|prefix| prefix.join("Caskroom").join(NAME).is_dir())
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
        .find(|link| links_to(link, target))
    {
        let homebrew = from_homebrew(&link);
        return State::Installed { link, homebrew };
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
    if link.symlink_metadata().is_err() {
        State::Missing { link }
    } else if scrawl_link(&link) {
        State::Stale { link }
    } else {
        State::Taken { link }
    }
}

/// Links `scrawl` and returns where. A link that's already right is left alone.
pub fn install(places: &Places) -> Result<PathBuf> {
    let target = places.target.as_ref();
    let link = match state(places) {
        State::Installed { link, .. } => return Ok(link),
        State::Missing { link } | State::Stale { link } => link,
        State::Taken { link } => bail!("{} is another program", tilde(&link)),
        State::Unavailable => bail!("the command-line tool works in the installed Scrawl.app"),
        State::Nowhere => bail!("there's no folder Scrawl can write to"),
    };
    let target = target.context("no Scrawl binary")?;
    let dir = link.parent().context("no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", tilde(dir)))?;
    // Made beside it and renamed over, so a stale link is replaced in one step.
    let temp = dir.join(format!(".{NAME}-{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    symlink(target, &temp).with_context(|| format!("couldn't write to {}", tilde(dir)))?;
    std::fs::rename(&temp, &link).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })?;
    Ok(link)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Removes the link Install made (or a stale one). Anything that isn't a link to Scrawl stays.
pub fn uninstall(places: &Places) -> Result<()> {
    let link = match state(places) {
        State::Installed { link, .. } | State::Stale { link } => link,
        _ => return Ok(()),
    };
    if scrawl_link(&link) {
        std::fs::remove_file(&link).with_context(|| format!("couldn't remove {}", tilde(&link)))?;
    }
    Ok(())
}

/// `~/.local/bin/scrawl` rather than `/Users/kit/.local/bin/scrawl`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

/// Whether a link here is likely on the user's PATH already: Homebrew's and /usr/local's bin
/// are on every macOS shell's PATH; a folder under home may not be.
pub fn usually_on_path(link: &Path) -> bool {
    link.parent().is_some_and(|dir| {
        dir == Path::new("/opt/homebrew/bin") || dir == Path::new("/usr/local/bin")
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "scrawl-cli-{name}-{}-{}",
                std::process::id(),
                crate::now_epoch_ms()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(std::fs::canonicalize(dir).unwrap())
        }

        /// A fake `Scrawl.app` at `name` with an executable `scrawl` and `scrawl-helper`.
        fn app(&self, name: &str) -> PathBuf {
            let macos = self.0.join(name).join("Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            for file in ["scrawl", "scrawl-helper"] {
                std::fs::write(macos.join(file), "#!/bin/sh\n").unwrap();
            }
            macos.join("scrawl")
        }

        fn places(&self, target: &Path, install: &[&str]) -> Places {
            Places {
                target: Some(target.to_owned()),
                install: install.iter().map(|d| self.0.join(d)).collect(),
                also: vec![self.0.join("home-bin")],
            }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn installs_into_the_first_writable_folder_and_uninstalls() {
        let sb = Sandbox::new("install");
        let target = sb.app("Scrawl.app");
        std::fs::create_dir_all(sb.0.join("brew/bin")).unwrap();
        // The first folder doesn't exist, so the second (which does) is used.
        let places = sb.places(&target, &["missing/bin", "brew/bin", "local/bin"]);
        let link = sb.0.join("brew/bin/scrawl");
        assert_eq!(state(&places), State::Missing { link: link.clone() });
        assert_eq!(install(&places).unwrap(), link);
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert_eq!(
            state(&places),
            State::Installed {
                link: link.clone(),
                homebrew: false
            }
        );
        // Installing again changes nothing.
        assert_eq!(install(&places).unwrap(), link);
        uninstall(&places).unwrap();
        assert!(link.symlink_metadata().is_err());
        assert_eq!(state(&places), State::Missing { link });
    }

    #[test]
    fn the_last_folder_is_created_when_nothing_else_is_writable() {
        let sb = Sandbox::new("local");
        let target = sb.app("Scrawl.app");
        let places = sb.places(&target, &["opt/bin", "local/bin"]);
        let link = sb.0.join("local/bin/scrawl");
        assert_eq!(state(&places), State::Missing { link: link.clone() });
        install(&places).unwrap();
        assert!(links_to(&link, &target));
        assert!(!usually_on_path(&link));
        assert!(usually_on_path(Path::new("/opt/homebrew/bin/scrawl")));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_is_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let sb = Sandbox::new("readonly");
        let target = sb.app("Scrawl.app");
        let locked = sb.0.join("locked/bin");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let places = sb.places(&target, &["locked/bin", "local/bin"]);
        let result = install(&places);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result.unwrap(), sb.0.join("local/bin/scrawl"));
    }

    #[test]
    fn another_program_called_scrawl_is_left_alone() {
        let sb = Sandbox::new("taken");
        let target = sb.app("Scrawl.app");
        let bin = sb.0.join("brew/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("scrawl"), "someone else's").unwrap();
        let places = sb.places(&target, &["brew/bin", "local/bin"]);
        assert_eq!(
            state(&places),
            State::Taken {
                link: bin.join("scrawl")
            }
        );
        assert!(install(&places).is_err());
        uninstall(&places).unwrap();
        assert_eq!(
            std::fs::read_to_string(bin.join("scrawl")).unwrap(),
            "someone else's"
        );
        // A symlink to some other tool counts as taken too.
        std::fs::remove_file(bin.join("scrawl")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/true", bin.join("scrawl")).unwrap();
        assert!(matches!(state(&places), State::Taken { .. }));
    }

    #[test]
    fn a_link_to_another_or_a_deleted_copy_is_replaced() {
        let sb = Sandbox::new("stale");
        let target = sb.app("Scrawl.app");
        let old = sb.app("Old/Scrawl.app");
        let bin = sb.0.join("brew/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&old, bin.join("scrawl")).unwrap();
        let places = sb.places(&target, &["brew/bin"]);
        assert_eq!(
            state(&places),
            State::Stale {
                link: bin.join("scrawl")
            }
        );
        install(&places).unwrap();
        assert_eq!(std::fs::read_link(bin.join("scrawl")).unwrap(), target);
        // A link whose app was deleted is stale as well.
        std::fs::remove_file(bin.join("scrawl")).unwrap();
        std::os::unix::fs::symlink(
            sb.0.join("Gone.app/Contents/MacOS/scrawl"),
            bin.join("scrawl"),
        )
        .unwrap();
        assert!(matches!(state(&places), State::Stale { .. }));
        uninstall(&places).unwrap();
        assert!(bin.join("scrawl").symlink_metadata().is_err());
    }

    #[test]
    fn existing_links_in_home_bin_and_from_homebrew_count_as_installed() {
        let sb = Sandbox::new("found");
        let target = sb.app("Scrawl.app");
        let home_bin = sb.0.join("home-bin");
        std::fs::create_dir_all(&home_bin).unwrap();
        // A link through a relative path still resolves to this binary.
        std::os::unix::fs::symlink(
            "../Scrawl.app/Contents/MacOS/scrawl",
            home_bin.join("scrawl"),
        )
        .unwrap();
        let places = sb.places(&target, &["brew/bin", "local/bin"]);
        assert_eq!(
            state(&places),
            State::Installed {
                link: home_bin.join("scrawl"),
                homebrew: false
            }
        );
        std::fs::remove_file(home_bin.join("scrawl")).unwrap();
        let bin = sb.0.join("brew/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(sb.0.join("brew/Caskroom/scrawl/0.4.0")).unwrap();
        std::os::unix::fs::symlink(&target, bin.join("scrawl")).unwrap();
        assert_eq!(
            state(&places),
            State::Installed {
                link: bin.join("scrawl"),
                homebrew: true
            }
        );
    }

    #[test]
    fn outside_the_app_there_is_nothing_to_link() {
        let places = Places {
            target: None,
            install: vec![std::env::temp_dir()],
            also: Vec::new(),
        };
        assert_eq!(state(&places), State::Unavailable);
        assert!(install(&places).is_err());
    }
}

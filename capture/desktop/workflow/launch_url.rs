//! Deliberately literal launch contract: no parsing, normalization or decoded commands.
pub fn valid(url: &str) -> bool {
    url == "apparent://open"
}

/// A new, fixed destination contract. No caller-supplied executable, path or callback.
pub fn hermes_request(url: &str) -> Option<&str> {
    let request = url.strip_prefix("apparent://hermes/")?;
    if request.len() > 38 { return None; }
    let parts: Vec<_> = request.split('/').collect();
    (parts.len() == 2 && parts[0].len() == 32
        && parts[0].bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && parts[1].bytes().all(|c| c.is_ascii_digit())
        && parts[1].parse::<u16>().is_ok_and(|port| (32768..=60999).contains(&port)))
        .then_some(request)
}

pub fn open_hermes(token: &str) -> anyhow::Result<()> {
    use std::process::{Command, Stdio};
    anyhow::ensure!(hermes_request(&format!("apparent://hermes/{token}")).is_some(), "Invalid Hermes request");
    let resources = super::electron::resolve()?;
    anyhow::ensure!(resources.root.join("dist/hermes-main.cjs").is_file()
        && resources.root.join("dist/hermes-preload.cjs").is_file(), "Hermes recording UI is missing; update Apparent");
    let mut command = Command::new(resources.executable);
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        command.arg("--ozone-platform=x11");
    }
    command.arg(resources.root.join("dist/hermes-main.cjs"))
        .arg("--hermes-request").arg(token)
        .arg("--apparent-executable").arg(std::env::current_exe()?)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(())
}

pub fn can_present(recording: bool, processing: bool, selecting: bool) -> bool {
    !recording && !processing && !selecting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_url_opens_the_app() {
        assert!(valid("apparent://open"));
    }

    #[test]
    fn hermes_launch_has_only_an_opaque_token() {
        let token = "0123456789abcdef0123456789abcdef/45000";
        assert_eq!(hermes_request(&format!("apparent://hermes/{token}")), Some(token));
        for suffix in ["", "../open", "0123456789ABCDEF0123456789ABCDEF", "0123456789abcdef0123456789abcdef?callback=https://evil"] {
            assert!(hermes_request(&format!("apparent://hermes/{suffix}")).is_none());
        }
    }

    #[test]
    fn launch_never_presents_over_active_capture_work() {
        assert!(can_present(false, false, false));
        for state in 1..8 {
            assert!(!can_present(state & 1 != 0, state & 2 != 0, state & 4 != 0));
        }
    }

    #[test]
    fn untrusted_variants_are_not_commands() {
        for url in [
            "",
            "apparent:",
            "apparent://",
            "apparent://open/",
            "apparent://OPEN",
            "APPARENT://open",
            "apparent://open?record=true",
            "apparent://open#capture",
            "apparent://record",
            "apparent://open/latest",
            "apparent://user@open",
            "apparent://open:80",
            "apparent://%6fpen",
            " apparent://open",
            "apparent://open\n",
            "apparent://open\0",
            "https://open",
            "file:///tmp/capture",
            "apparent://open?",
            "apparent://open#",
            "apparent://open;sh",
            "apparent://open%00",
            "apparent://оpen",
        ] {
            assert!(!valid(url), "accepted {url:?}");
        }
        assert!(!valid(&format!("apparent://open{}", "x".repeat(1_000_000))));
    }
}

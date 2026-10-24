//! The running build's identity, and whether it takes beta updates. A build never knows its
//! channel: every build of a version is a candidate with a plain version and its own build number,
//! "beta" is a property of the appcast item, and `"updateChannel": "beta"` opts this Mac in.

pub fn beta_enabled(channel: Option<&str>) -> bool {
    channel == Some("beta")
}

/// `"updateChannel": "beta"` in `config.json`, read fresh: Settings can change it at any time.
#[cfg(target_os = "macos")]
pub fn beta_updates() -> bool {
    beta_enabled(crate::handoff::update_channel().as_deref())
}

pub fn version() -> String {
    bundle_value("CFBundleShortVersionString")
        .unwrap_or_else(|| env!("SCRAWL_RELEASE_VERSION").to_owned())
}

pub fn build() -> String {
    bundle_value("CFBundleVersion").unwrap_or_else(|| env!("SCRAWL_RELEASE_BUILD").to_owned())
}

/// `0.3.14 (18)`, plus `· Beta updates on` when this Mac takes betas.
pub fn display(beta: bool) -> String {
    label(&version(), &build(), beta)
}

fn label(version: &str, build: &str, beta: bool) -> String {
    let suffix = if beta { " · Beta updates on" } else { "" };
    format!("{version} ({build}){suffix}")
}

/// The bundle's own identity. `install-app.sh` marks dev installs `ScrawlDevBuild`: their bundle
/// keeps build 1 (so Sparkle offers them any release, as before) but they show Cargo's version
/// and `(dev)`.
#[cfg(target_os = "macos")]
fn bundle_value(key: &str) -> Option<String> {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::mainBundle();
    if bundle
        .objectForInfoDictionaryKey(&NSString::from_str("ScrawlDevBuild"))
        .is_some()
    {
        return None;
    }
    bundle
        .objectForInfoDictionaryKey(&NSString::from_str(key))
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|value| value.to_string())
}

#[cfg(not(target_os = "macos"))]
fn bundle_value(_: &str) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
pub fn report_problem() {
    let os = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    let body = format!(
        "Scrawl {}\nBuild: {}\nmacOS: {os}\n\nWhat happened?\n\nSteps to reproduce:\n\nExpected behavior:\n",
        version(),
        build()
    );
    let encoded: String = body.bytes().map(|b| format!("%{b:02X}")).collect();
    let url = format!(
        "https://github.com/kitlangton/scrawl/issues/new?labels=beta&title=%5BBeta%5D%20&body={encoded}"
    );
    let _ = std::process::Command::new("open").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_requires_explicit_opt_in() {
        for value in [
            None,
            Some("stable"),
            Some("Beta"),
            Some("nightly"),
            Some(""),
        ] {
            assert!(!beta_enabled(value));
        }
        assert!(beta_enabled(Some("beta")));
    }

    #[test]
    fn the_label_names_the_setting_not_the_build() {
        assert_eq!(label("0.3.14", "18", false), "0.3.14 (18)");
        assert_eq!(label("0.3.14", "18", true), "0.3.14 (18) · Beta updates on");
    }
}

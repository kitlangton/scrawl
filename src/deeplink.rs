//! `scrawl://` links, so other apps (launchers like Raycast, Shortcuts, scripts) can do what the
//! menu bar does. Names follow CleanShot X's URL scheme where the action is the same
//! (`record-screen`, `all-in-one`, `open-settings?tab=`); the others are the menu item's or
//! picker mode's name in the same form. Parsing is pure; `ui::run_app` turns a link into a
//! `MenuAction` and runs it exactly as the menu would.

use crate::settings::Pane;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The menu's Record Screen: start a capture of the display under the pointer, or finish
    /// the one running.
    RecordScreen,
    /// The menu's Choose What to Record…: the picker on the mode last used.
    AllInOne,
    /// The picker on Area.
    RecordArea,
    /// The picker on Window.
    RecordWindow,
    /// Capture the cabled iPhone or iPad.
    RecordDevice,
    /// The menu's Open Latest Capture.
    OpenLatestCapture,
    /// The menu's Settings…, on a pane when `?tab=` names one.
    OpenSettings(Option<Pane>),
}

/// Whether a link may run. Any app or web page can open a link, so the ones that start or
/// finish a recording by themselves wait for Settings › Agents › "Let other apps record"
/// (`recordingLinks`, off by default). Picker links still need a click or a drag to record, and
/// the rest open windows.
pub fn allowed(link: Link, recording_links: bool) -> bool {
    recording_links || !matches!(link, Link::RecordScreen | Link::RecordDevice)
}

/// Reads one `scrawl://` URL. Case doesn't matter, a trailing `/` is allowed, and unknown query
/// parameters are ignored (as CleanShot does). An unknown `tab` opens Settings on its usual pane.
pub fn parse(url: &str) -> Result<Link, String> {
    let rest = strip_scheme(url.trim()).ok_or_else(|| format!("not a scrawl:// link: {url}"))?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let command = path.trim_matches('/').to_ascii_lowercase();
    let link = match command.as_str() {
        "record-screen" => Link::RecordScreen,
        "all-in-one" => Link::AllInOne,
        "record-area" => Link::RecordArea,
        "record-window" => Link::RecordWindow,
        "record-device" => Link::RecordDevice,
        "open-latest-capture" => Link::OpenLatestCapture,
        "open-settings" => Link::OpenSettings(query_value(query, "tab").and_then(pane)),
        "" => return Err("scrawl:// link names no command".into()),
        other => return Err(format!("unknown scrawl:// command: {other}")),
    };
    Ok(link)
}

fn strip_scheme(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("scrawl") {
        return None;
    }
    Some(rest.strip_prefix("//").unwrap_or(rest))
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn pane(name: &str) -> Option<Pane> {
    match name.to_ascii_lowercase().as_str() {
        "general" => Some(Pane::General),
        "recording" => Some(Pane::Recording),
        "storage" => Some(Pane::Storage),
        "permissions" => Some(Pane::Permissions),
        "agents" => Some(Pane::Agents),
        "about" => Some(Pane::About),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_parses() {
        let cases = [
            ("scrawl://record-screen", Link::RecordScreen),
            ("scrawl://all-in-one", Link::AllInOne),
            ("scrawl://record-area", Link::RecordArea),
            ("scrawl://record-window", Link::RecordWindow),
            ("scrawl://record-device", Link::RecordDevice),
            ("scrawl://open-latest-capture", Link::OpenLatestCapture),
            ("scrawl://open-settings", Link::OpenSettings(None)),
            (
                "scrawl://open-settings?tab=recording",
                Link::OpenSettings(Some(Pane::Recording)),
            ),
            (
                "scrawl://open-settings?tab=agents",
                Link::OpenSettings(Some(Pane::Agents)),
            ),
        ];
        for (url, want) in cases {
            assert_eq!(parse(url), Ok(want), "{url}");
        }
    }

    #[test]
    fn forgiving_about_form() {
        assert_eq!(parse("SCRAWL://Record-Screen/"), Ok(Link::RecordScreen));
        assert_eq!(parse("scrawl:record-area"), Ok(Link::RecordArea));
        assert_eq!(parse("  scrawl://all-in-one  "), Ok(Link::AllInOne));
        assert_eq!(
            parse("scrawl://record-window?display=2"),
            Ok(Link::RecordWindow)
        );
        assert_eq!(
            parse("scrawl://open-settings?x=1&TAB=About"),
            Ok(Link::OpenSettings(Some(Pane::About)))
        );
    }

    #[test]
    fn unknown_tab_opens_settings_anyway() {
        assert_eq!(
            parse("scrawl://open-settings?tab=wallpaper"),
            Ok(Link::OpenSettings(None))
        );
        assert_eq!(
            parse("scrawl://open-settings?tab"),
            Ok(Link::OpenSettings(None))
        );
    }

    #[test]
    fn rejects_what_it_does_not_know() {
        assert!(parse("scrawl://").is_err());
        assert!(parse("scrawl://record").is_err());
        assert!(parse("scrawl://capture-area").is_err());
        assert!(parse("cleanshot://record-screen").is_err());
        assert!(parse("https://scrawl.kitlangton.com/record-screen").is_err());
        assert!(parse("record-screen").is_err());
    }

    #[test]
    fn only_recording_links_need_the_setting() {
        let all = [
            Link::RecordScreen,
            Link::AllInOne,
            Link::RecordArea,
            Link::RecordWindow,
            Link::RecordDevice,
            Link::OpenLatestCapture,
            Link::OpenSettings(None),
            Link::OpenSettings(Some(Pane::Permissions)),
        ];
        for link in all {
            assert!(allowed(link, true), "{link:?} with recordingLinks on");
            let records = matches!(link, Link::RecordScreen | Link::RecordDevice);
            assert_eq!(
                allowed(link, false),
                !records,
                "{link:?} with recordingLinks off"
            );
        }
    }
}

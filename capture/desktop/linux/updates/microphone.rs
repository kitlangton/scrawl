//! PulseAudio-compatible sources (including PipeWire's Pulse server). CPAL's ALSA host
//! exposes only `default`/`pipewire` on these desktops, not the individual inputs.
//! Keep source selection per stream; never change the user's system default.

const PREFIX: &str = "pulse:";

pub fn source(id: &str) -> Option<&str> {
    id.strip_prefix(PREFIX).filter(|name| !name.is_empty())
}

pub fn inputs() -> Option<crate::capture::engine::runtime::audio::mic::Inputs> {
    let output = std::process::Command::new("pactl")
        .args(["--format=json", "list", "sources"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let devices = parse_sources(&output.stdout)?;
    if devices.is_empty() {
        return None;
    }
    let default = std::process::Command::new("pactl")
        .arg("get-default-source")
        .output()
        .ok()
        .filter(|result| result.status.success())
        .and_then(|result| String::from_utf8(result.stdout).ok())
        .map(|name| format!("{PREFIX}{}", name.trim()));
    Some(crate::capture::engine::runtime::audio::mic::Inputs { devices, default })
}

fn parse_sources(bytes: &[u8]) -> Option<Vec<crate::capture::engine::runtime::audio::mic::Input>> {
    let sources: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let sources = sources.as_array()?;
    Some(
        sources
            .iter()
            .filter_map(|source| {
                let name = source.get("name")?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                Some(crate::capture::engine::runtime::audio::mic::Input {
                    id: format!("{PREFIX}{name}"),
                    name: source
                        .get("description")
                        .and_then(|d| d.as_str())
                        .filter(|d| !d.is_empty())
                        .unwrap_or(name)
                        .to_owned(),
                })
            })
            .collect(),
    )
}

pub use apparent_capture_engine::linux::microphone::Stream;

#[cfg(test)]
mod tests {
    #[test]
    fn pulse_source_ids_keep_distinct_inputs_and_readable_names() {
        let devices = super::parse_sources(br#"[{"name":"mic_a.monitor","description":"Monitor of MicA"},{"name":"mic_b.monitor","description":"Monitor of MicB"}]"#).unwrap();
        assert_eq!(devices[0].id, "pulse:mic_a.monitor");
        assert_eq!(devices[1].name, "Monitor of MicB");
        assert_eq!(super::source(&devices[1].id), Some("mic_b.monitor"));
        assert_eq!(super::source("default"), None);
    }
}

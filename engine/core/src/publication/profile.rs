//! Full apparent-bundle/1 manifest profile validation.
use super::{Entry, validate_relative};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) fn validate_manifest(
    manifest: &Value,
    bundle_id: &str,
    entries: &BTreeMap<&str, &Entry>,
) -> Result<()> {
    let object = manifest
        .as_object()
        .context("bundle manifest must be an object")?;
    for required in [
        "version",
        "id",
        "createdAtMs",
        "durationMs",
        "target",
        "transcript",
        "strokes",
        "marks",
        "events",
        "keyframes",
        "contactSheet",
    ] {
        ensure!(
            object.contains_key(required),
            "v1 bundle manifest missing {required}"
        );
    }
    ensure!(
        manifest["version"].as_u64() == Some(1) && manifest["id"].as_str() == Some(bundle_id),
        "bundle manifest version or identity mismatch"
    );
    ensure!(
        manifest["createdAtMs"].as_u64().is_some() && manifest["durationMs"].as_u64().is_some(),
        "invalid manifest clock"
    );
    for optional_text in ["video", "audio", "source", "note", "imageQualityWarning"] {
        ensure!(
            manifest.get(optional_text).is_none_or(Value::is_null)
                || manifest[optional_text].as_str().is_some(),
            "invalid optional manifest field {optional_text}"
        );
    }
    ensure!(
        manifest
            .get("videoOffsetMs")
            .is_none_or(|v| v.as_u64().is_some()),
        "invalid video offset"
    );
    ensure!(
        manifest["target"].is_object()
            && manifest["transcript"].is_object()
            && manifest["strokes"].is_array()
            && manifest["marks"].is_array()
            && manifest["events"].is_array(),
        "invalid v1 manifest sections"
    );
    validate_target(&manifest["target"])?;
    validate_transcript(&manifest["transcript"])?;
    validate_optional_manifest_sections(manifest)?;
    validate_strokes(&manifest["strokes"])?;
    validate_marks(&manifest["marks"])?;
    validate_events(&manifest["events"])?;
    let frames = manifest["keyframes"]
        .as_array()
        .context("keyframes must be an array")?;
    ensure!(!frames.is_empty(), "bundle has no keyframes");
    let contact = manifest["contactSheet"]
        .as_str()
        .context("contact sheet reference missing")?;
    require_role(entries, contact, "contact-sheet")?;
    let mut indices = BTreeSet::new();
    for frame in frames {
        let index = frame["index"].as_u64().context("keyframe index missing")?;
        ensure!(
            index > 0 && indices.insert(index),
            "duplicate/zero keyframe index"
        );
        let path = frame["file"]
            .as_str()
            .context("keyframe file reference missing")?;
        ensure!(
            frame["t"].as_u64().is_some() && frame["event"].as_str().is_some(),
            "incomplete keyframe metadata"
        );
        ensure!(
            [
                "mark",
                "click",
                "laser",
                "scene_change",
                "speech",
                "initial",
                "whiteboard"
            ]
            .contains(&frame["reason"].as_str().unwrap_or_default()),
            "unknown keyframe reason"
        );
        for field in ["said", "markId"] {
            ensure!(
                frame.get(field).is_none_or(Value::is_null) || frame[field].as_str().is_some(),
                "invalid optional keyframe field {field}"
            );
        }
        ensure!(
            frame.get("cursor").is_none_or(Value::is_null)
                || frame["cursor"]
                    .as_array()
                    .is_some_and(|v| v.len() == 2
                        && v.iter().all(|n| n.as_f64().is_some_and(f64::is_finite))),
            "invalid keyframe cursor"
        );
        require_role(entries, path, "frame")?;
    }
    for (field, role) in [("video", "video"), ("audio", "audio")] {
        if let Some(path) = manifest[field].as_str() {
            require_role(entries, path, role)?;
        }
    }
    if let Some(crop) = manifest.get("autocrop").filter(|v| !v.is_null()) {
        ensure!(crop["label"].as_str().is_some(), "crop label missing");
        let dir = crop["fullFrames"]
            .as_str()
            .context("full frame directory missing")?;
        validate_relative(dir)?;
        let sheet = crop["fullContactSheet"]
            .as_str()
            .context("full contact sheet missing")?;
        require_role(entries, sheet, "contact-sheet")?;
        let crops = crop["crops"].as_array().context("crop metadata missing")?;
        ensure!(crops.len() == frames.len(), "incomplete crop metadata");
        for item in crops {
            ensure!(item["index"].as_u64().is_some(), "crop index missing");
            for field in ["x", "y", "width", "height"] {
                ensure!(
                    item[field].as_f64().is_some_and(f64::is_finite),
                    "invalid crop field {field}"
                );
            }
            ensure!(
                item["width"].as_f64().unwrap() > 0.0 && item["height"].as_f64().unwrap() > 0.0,
                "empty crop rectangle"
            );
            ensure!(
                ["window", "windows", "focus", "activity", "fallback", "full"]
                    .contains(&item["kind"].as_str().unwrap_or_default())
                    && item["reason"].as_str().is_some()
                    && item.get("apps").is_none_or(|apps| apps
                        .as_array()
                        .is_some_and(|apps| apps.iter().all(Value::is_string))),
                "invalid crop metadata"
            );
        }
        if let Some(changes) = crop.get("changes") {
            let valid_changes = changes.as_array().is_some_and(|items| {
                items.iter().all(|rects| {
                    rects.as_array().is_some_and(|rects| {
                        rects.iter().all(|rect| {
                            rect.as_array().is_some_and(|coords| {
                                coords.len() == 4
                                    && coords
                                        .iter()
                                        .all(|n| n.as_f64().is_some_and(f64::is_finite))
                            })
                        })
                    })
                })
            });
            ensure!(valid_changes, "invalid crop changes");
        }
        for frame in frames {
            let leaf = Path::new(frame["file"].as_str().unwrap())
                .file_name()
                .and_then(|f| f.to_str())
                .context("invalid full-frame leaf")?;
            require_role(entries, &format!("{dir}/{leaf}"), "frame")?;
        }
    }
    Ok(())
}
fn validate_target(target: &Value) -> Result<()> {
    ensure!(
        ["display", "window", "region", "device"]
            .contains(&target["mode"].as_str().unwrap_or_default()),
        "unknown capture target mode"
    );
    ensure!(
        target["displayId"].as_u64().is_some(),
        "target display ID missing"
    );
    let frame = &target["frame"];
    for key in ["x", "y", "width", "height", "scale"] {
        ensure!(
            frame[key].as_f64().is_some_and(f64::is_finite),
            "invalid target frame {key}"
        );
    }
    ensure!(
        frame["width"].as_f64().unwrap() > 0.0
            && frame["height"].as_f64().unwrap() > 0.0
            && frame["scale"].as_f64().unwrap() > 0.0,
        "empty target frame"
    );
    for key in ["app", "bundleId", "windowTitle"] {
        ensure!(
            target.get(key).is_none_or(Value::is_null) || target[key].as_str().is_some(),
            "invalid target {key}"
        );
    }
    ensure!(
        target.get("windowId").is_none_or(Value::is_null) || target["windowId"].as_u64().is_some(),
        "invalid target window ID"
    );
    Ok(())
}
fn validate_optional_manifest_sections(manifest: &Value) -> Result<()> {
    if let Some(value) = manifest
        .get("interactionTelemetry")
        .filter(|v| !v.is_null())
    {
        ensure!(
            value.as_str() == Some("videoOnly"),
            "unknown interaction telemetry mode"
        );
    }
    if let Some(context) = manifest.get("context") {
        ensure!(
            context
                .as_object()
                .is_some_and(|map| map.values().all(Value::is_string)),
            "invalid context map"
        );
    }
    if let Some(changes) = manifest.get("contextChanges") {
        ensure!(
            changes
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v["t"].as_u64().is_some()
                    && v["key"].as_str().is_some()
                    && (v.get("value").is_none_or(Value::is_null)
                        || v["value"].as_str().is_some()))),
            "invalid context changes"
        );
    }
    if let Some(screens) = manifest.get("screens") {
        ensure!(
            screens
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v["t0"].as_u64().is_some()
                    && v["t1"].as_u64().is_some()
                    && v["name"].as_str().is_some())),
            "invalid screen spans"
        );
    }
    if let Some(windows) = manifest.get("windows") {
        ensure!(
            windows
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v["t"].as_u64().is_some()
                    && v["windows"]
                        .as_array()
                        .is_some_and(|entries| entries.iter().all(|w| w["id"]
                            .as_u64()
                            .is_some()
                            && w["app"].as_str().is_some()
                            && w["bounds"]
                                .as_array()
                                .is_some_and(|b| b.len() == 4 && b.iter().all(Value::is_i64))
                            && w["layer"].as_i64().is_some()
                            && w.get("regular").is_none_or(Value::is_boolean)
                            && w.get("frontmost").is_none_or(Value::is_boolean)
                            && w.get("title").is_none_or(Value::is_string)
                            && w.get("bundleId").is_none_or(Value::is_string))))),
            "invalid window snapshots"
        );
    }
    if let Some(cursor) = manifest.get("cursor") {
        ensure!(
            cursor
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v.as_array().is_some_and(
                    |sample| sample.len() == 3
                        && sample[0].as_u64().is_some()
                        && sample[1].as_f64().is_some_and(f64::is_finite)
                        && sample[2].as_f64().is_some_and(f64::is_finite)
                ))),
            "invalid cursor samples"
        );
    }
    if let Some(device) = manifest.get("device").filter(|v| !v.is_null()) {
        ensure!(
            device.is_object()
                && device["name"].as_str().is_some()
                && device["capture"].as_str().is_some()
                && device["width"].as_u64().is_some()
                && device["height"].as_u64().is_some(),
            "invalid device metadata"
        );
        for field in [
            "id",
            "model",
            "system",
            "app",
            "appVersion",
            "appShortVersion",
        ] {
            ensure!(
                device.get(field).is_none_or(Value::is_null) || device[field].as_str().is_some(),
                "invalid device string field {field}"
            );
        }
        if let Some(screen) = device.get("screen").filter(|v| !v.is_null()) {
            ensure!(
                screen
                    .as_array()
                    .is_some_and(|v| v.len() == 3
                        && v.iter().all(|n| n.as_f64().is_some_and(f64::is_finite))),
                "invalid device screen bounds"
            );
        }
    }
    if let Some(destination) = manifest.get("destination").filter(|v| !v.is_null()) {
        ensure!(
            destination.is_object()
                && destination["session"].as_str().is_some()
                && destination["via"].as_str().is_some(),
            "invalid destination metadata"
        );
        for field in ["title", "url", "directory"] {
            ensure!(
                destination.get(field).is_none_or(Value::is_null)
                    || destination[field].as_str().is_some(),
                "invalid destination string field {field}"
            );
        }
    }
    Ok(())
}
fn validate_transcript(transcript: &Value) -> Result<()> {
    ensure!(
        transcript["source"].as_str().is_some() && transcript["text"].as_str().is_some(),
        "transcript source/text missing"
    );
    let segments = transcript["segments"]
        .as_array()
        .context("transcript segments missing")?;
    for segment in segments {
        ensure!(
            segment["t0"].as_u64().is_some()
                && segment["t1"].as_u64().is_some()
                && segment["text"].as_str().is_some(),
            "invalid transcript segment"
        );
    }
    if let Some(words) = transcript.get("words").and_then(Value::as_array) {
        for word in words {
            ensure!(
                word["t0"].as_u64().is_some()
                    && word["t1"].as_u64().is_some()
                    && word["text"].as_str().is_some(),
                "invalid word timing"
            );
        }
    }
    ensure!(
        transcript.get("estimated").is_none_or(Value::is_boolean),
        "invalid transcript estimated flag"
    );
    ensure!(
        transcript.get("model").is_none_or(Value::is_null)
            || transcript["model"].as_str().is_some(),
        "invalid transcript model"
    );
    if let Some(missing) = transcript.get("missing").filter(|v| !v.is_null()) {
        ensure!(
            missing["reason"].as_str().is_some()
                && missing.get("retry").is_none_or(Value::is_boolean),
            "invalid missing transcript detail"
        );
    }
    Ok(())
}
fn validate_strokes(strokes: &Value) -> Result<()> {
    for stroke in strokes.as_array().unwrap() {
        ensure!(
            stroke["t0"].as_u64().is_some()
                && stroke["t1"].as_u64().is_some()
                && stroke["fw"].as_u64().is_some()
                && stroke["fh"].as_u64().is_some(),
            "invalid stroke frame/timing"
        );
        ensure!(
            stroke["pts"]
                .as_array()
                .is_some_and(|points| points.len() % 3 == 0 && points.iter().all(Value::is_i64)),
            "invalid stroke point triples"
        );
        ensure!(
            stroke.get("laser").is_none_or(Value::is_boolean),
            "invalid stroke laser flag"
        );
        if let Some(rgb) = stroke.get("rgb").filter(|value| !value.is_null()) {
            ensure!(
                rgb.as_array().is_some_and(
                    |v| v.len() == 3 && v.iter().all(|n| n.as_u64().is_some_and(|x| x <= 255))
                ),
                "invalid stroke RGB tuple"
            );
        }
    }
    Ok(())
}
fn validate_marks(marks: &Value) -> Result<()> {
    for mark in marks.as_array().unwrap() {
        ensure!(
            mark["id"].as_str().is_some()
                && mark["description"].as_str().is_some()
                && mark["t0"].as_u64().is_some()
                && mark["t1"].as_u64().is_some(),
            "invalid mark fields"
        );
        ensure!(
            mark["strokes"]
                .as_array()
                .is_some_and(|values| values.iter().all(|value| value.as_u64().is_some())),
            "invalid mark stroke references"
        );
        ensure!(
            mark["boxNorm"].as_array().is_some_and(
                |v| v.len() == 4 && v.iter().all(|n| n.as_f64().is_some_and(f64::is_finite))
            ),
            "invalid mark bounds"
        );
        ensure!(
            [
                "circle",
                "underline",
                "line",
                "arrow",
                "tap",
                "pin",
                "mark",
                "flag"
            ]
            .contains(&mark["shape"].as_str().unwrap_or_default()),
            "invalid mark shape"
        );
    }
    Ok(())
}
fn validate_events(events: &Value) -> Result<()> {
    for event in events.as_array().unwrap() {
        let kind = event["kind"]
            .as_str()
            .context("interaction event kind missing")?;
        ensure!(
            event["t"].as_u64().is_some(),
            "interaction event time missing"
        );
        match kind {
            "note" => ensure!(
                event["pin"].as_u64().is_some()
                    && event["x"].as_f64().is_some_and(f64::is_finite)
                    && event["y"].as_f64().is_some_and(f64::is_finite)
                    && event["text"].as_str().is_some(),
                "invalid note event"
            ),
            "click" => ensure!(
                event["x"].as_f64().is_some_and(f64::is_finite)
                    && event["y"].as_f64().is_some_and(f64::is_finite),
                "invalid click event"
            ),
            "drag" => ensure!(
                event["t1"].as_u64().is_some()
                    && ["x0", "y0", "x1", "y1"]
                        .iter()
                        .all(|key| event[*key].as_f64().is_some_and(f64::is_finite)),
                "invalid drag event"
            ),
            "scroll" => ensure!(
                ["x", "y", "dx", "dy"]
                    .iter()
                    .all(|key| event[*key].as_f64().is_some_and(f64::is_finite)),
                "invalid scroll event"
            ),
            "point" => ensure!(
                event["x"].as_f64().is_some_and(f64::is_finite)
                    && event["y"].as_f64().is_some_and(f64::is_finite),
                "invalid point event"
            ),
            "key" => ensure!(event["shortcut"].as_str().is_some(), "invalid key event"),
            "app_change" => ensure!(event["app"].as_str().is_some(), "invalid app-change event"),
            "app_event" => ensure!(event["label"].as_str().is_some(), "invalid app event"),
            _ => bail!("unknown interaction event kind"),
        }
    }
    Ok(())
}
fn require_role(entries: &BTreeMap<&str, &Entry>, path: &str, role: &str) -> Result<()> {
    validate_relative(path)?;
    ensure!(
        entries.get(path).is_some_and(|entry| entry.role == role),
        "bundle reference is missing or has wrong inventory role: {path}"
    );
    Ok(())
}

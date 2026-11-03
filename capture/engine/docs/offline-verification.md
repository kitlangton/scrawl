# Offline processing acceptance

`cargo test --offline offline_pipeline -- --nocapture` runs synthetic acquisition
inputs through `build_bundle_from_parts`, the production entry shared by screen,
phone and recovery callers. It runs on Linux without a display server, microphone,
Apple helper, speech model, external service or private recording.

The fixture supplies small RGBA frames, a click, measured-style word timings and
session metadata. It reloads the published manifest and summary, checks transcript
and event preservation, decodes the produced PNGs and verifies both source scenes
survive selection/rendering. Set `APPARENT_OFFLINE_EVIDENCE` to a fresh absolute
bundle path to retain text/images for inspection; otherwise the test removes its
own temporary output. The transcript is synthetic input, not recognizer evidence.

Existing coverage includes selector unit tests in `keyframes.rs`, publication and
decoder-fallback regressions in `bundle/bundle_processing/`, recovery tests with
injected media in `bundle/recover.rs`, audio persistence subprocess tests, and the
Desktop demo producer. This test adds a compact saved-output acceptance case at
the shared production boundary; it does not replace those focused tests.

Native acquisition, real decoder behavior, speech recognition quality/timing,
permissions, overlays, clipboard and OS integration still require their existing
platform checks. This test does not establish Mac native behavior or Linux GUI
acceptance and changes no release gate or product promise.

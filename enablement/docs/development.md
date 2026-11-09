# Development quick reference

[← Apparent](../../README.md)

## Helper

`apparent-helper` is a small Swift executable built by `build.rs` from explicit owner-native source groups and shipped inside
`Apparent.app` next to `apparent`. It owns the Swift-only or unbound Apple frameworks: SpeechAnalyzer,
CoreMediaIO/AVFoundation iPhone capture with the mirror window, AVFoundation movie decoding, and the
SwiftUI welcome window (`onboarding`).
Apparent runs it as a subprocess (JSON on stdout; `stop`/`cancel` on stdin for the mirror). Run it
directly to test a piece in isolation:

```sh
target/debug/apparent-helper devices
target/debug/apparent-helper permissions
target/debug/apparent-helper transcribe audio.wav
target/debug/apparent-helper frames recording.mov --out-dir /tmp/f --at 1000,2000 --max-edge 1600
target/debug/apparent-helper shrink recording.mov --out /tmp/small.mov --max-edge 1600
target/debug/apparent-helper export plan.json   # the plan `apparent export` writes (capture/engine/export)
target/debug/apparent-helper permission-panel --permission screen-recording --preview   # the drag card, no grant polling
```

`permission-panel --permission screen-recording|accessibility [--from X,Y,W,H]`
(`--from` is the button's frame in global top-left points) prints JSON lines: `docked`, `dropped`,
then `granted` or `cancelled` with a `reason` (`back`, `settings-closed`, `no-settings-window`,
`parent`, `timeout`), and exits. On stdin it takes `cancel`, `back` and `granted`.
`permission-panel --check P` prints `{"granted":…}` from a fresh process. `permission-panel --plan --settings
X,Y,W,H --visible X,Y,W,H` prints where the card would dock (`below`, `inside`, `right`, `left` or
`clamped`); `capture/desktop/scripts/support/check-permission-panel-dock.sh` checks the cases.

## Testing

```sh
taskpolicy -c utility cargo test -j 3 --offline -- --test-threads=2
./capture/desktop/scripts/packaging/install-app.sh
```

On Linux, run `cargo test`; then run `apparent app` in a desktop session and record with `Alt+Shift+S`.
`APPARENT_SHOW_OWN_WINDOWS=1` keeps Apparent's own windows (capsule, overlay, picker) in screenshots,
which normally leave them out.

- **iPhone:** connect a phone, press `⌥⇧S`, click the iPhone chip, talk, draw over the mirror, press M,
  Finish. Inspect the bundle's `summary.md`, `apparent.json` (`source`, `device`), contact sheet, and
  frames.
- **Transcription:** `apparent-helper transcribe <wav or movie>` prints words with timings;
  `apparent reprocess <bundle> --out …` reruns transcription and keyframe selection on a real bundle.

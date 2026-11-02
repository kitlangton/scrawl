# Onboarding, permissions and Settings

Read this before changing `capture/desktop/settings/onboarding.rs`, `capture/desktop/settings/settings.rs`, `composition/command_line.rs` or the helper's `Onboarding.swift`, `PermissionPanel.swift` and `Settings.swift`.

## Onboarding (`capture/desktop/settings/onboarding.rs`, `capture/desktop/native/Onboarding.swift`)

Rust decides what shows (`Flow`, `launch_plan`, `Gate`, tested) and stores progress under `"onboarding"` in `~/.config/apparent/config.json`, including the tour's current step (`resume`), so any quit (macOS's "Quit & Reopen" too) picks up there; the SwiftUI window in `apparent-helper onboarding` only renders the state it's sent and reports buttons (`grant:<permission>` for a row).

Tour: Welcome → HowItWorks → Permissions (one checklist of three required rows, Screen Recording first, then Microphone and Accessibility; any order works) → Shortcut → TryIt → Done. Shortcut shows the effective Record keys. Continue moves on without editing; Change shortcut starts the Record recorder in the same onboarding page, using Settings' validation, save, cancel and reset behavior. Leaving the page cancels an unfinished recording and resumes hotkeys. On Omarchy, a Hyprland-owned shortcut cannot be replaced by Apparent; the inline page explains that ownership. Camera (iPhone) is never in the tour: its first use opens `Flow::fix(vec![Camera])`, the same screen with one row.

HowItWorks is a silent 11 s loop (`capture/desktop/native/OnboardingDemo.swift`): a pretend checkout page whose Pay button runs out of its window, ⌥⇧S opening the pill, subtitles for what the person says, a ⇧ mark around the bug, a click, ⌥⇧S to Copied, the frame lifting off as a one-frame contact sheet with its `1` chip, ⌥⇧V into an agent chat, and the fix. Everything on the stage is a function of one clock (`DemoScript`), so any moment renders the same. Holding ⇧ over the stage pauses the loop and draws a mark in the window (nothing is recorded or asked for). The button under it reads Skip until the loop has played through or a mark was drawn, then Continue; the helper reports `skip`, `watched` and `practiced`, and `Flow` keeps the furthest outcome in `Progress::tutorial` (`"tutorial"`), so a resumed tour keeps Continue. Reduced motion shows four still frames with captions. `apparent-helper onboarding --render DIR --demo-frames 30` writes one loop as PNGs for a video.

Input Monitoring is never asked for or read (macOS wants a relaunch after it; the enum variant stays only so 0.3.x configs load, and `input_monitoring_is_never_requested` greps the sources): clicks come from an `NSEvent` global mouse monitor (no permission), ⇧ and held modifiers from `CGEventSourceFlagsState` (no permission), keys from an active `kCGEventTapOptionDefault` tap (Accessibility), which the HID poller makes or stops every 500 ms as `AXIsProcessTrusted` changes (`check_key_tap`; live, no relaunch). Never poll key codes (`CGEventSourceKeyState`) or make a listen-only tap.

Grants are re-read every 0.5 s from a fresh `apparent-helper permissions`; the coordinator caches them (`ApparentCoordinator::grants`). On macOS the separately signed helper can disagree with the app: Accessibility's effective status comes from `AXIsProcessTrusted` in the recording process, matching the key tap and recording gate. Screen Recording still uses the fresh helper to detect new grants and revoked grants; a cached positive preflight in the running app must not override a fresh denial. The recording gate checks both that latest grant and this process's screen usability, so a revoked grant blocks a new capture even if macOS lets the old process record until it quits. Linux's portal permission behavior is unchanged.

On macOS one background poller runs for the app's lifetime, including when Settings and onboarding are closed. It delivers each successful read to the coordinator and both open windows; those windows do not start duplicate permission pollers. A revocation reaches the recording gate on the next successful refresh (500 ms between reads), without waiting for a window to reopen or running the helper on the UI thread. Linux keeps its existing window-owned polling and portal admission behavior.

macOS shows "Quit & Reopen / Later" the moment Screen Recording is granted, and a running process may not see the grant (`CGPreflightScreenCaptureAccess` caches). So once the helper says granted, `read_grants` tries a one-frame ScreenCaptureKit capture in this process (`capture::screen_capture_usable(true)`); if it fails, `Flow::tick` shows "Reopening Apparent" for `RELAUNCH_AFTER_MS` and returns `Effect::Relaunch`, and the coordinator saves `Progress::relaunching` (tour `resume` = Permissions, or `reopenFix`) and `onboarding::relaunch()` reopens the app (without `-n`) after this pid exits. Quit & Reopen lands in the same place through the saved `resume`; a tour resumed on Permissions stays there. `relaunchedAt` stops a second automatic relaunch within `RELAUNCH_GUARD_MS`; the row then offers Reopen Apparent. `apparent sck-probe [--watch S]` / `apparent-helper sck-probe` test whether a grant reaches a running process.

Until the required ones are on, ⌥⇧S and the menu's record items start nothing (no picker or overlay; the picker sits at `NSStatusWindowLevel + 1` and takes clicks, which trapped macOS prompts under it): `Gate` holds the request and `tick_gate` opens or raises the window once the modifiers are up. Capture code must not call `CGRequestScreenCaptureAccess` or start the iPhone mirror without Camera: go through `ready_to_record` / `ready_for`.

When a row turns on, the window comes forward once macOS stops asking (no UserNotificationCenter or SecurityAgent window, no System Settings alert in front such as its Quit & Reopen; `returnAfterGrant`, for five minutes after the grant). Once Screen Recording works on Permissions or Try it, `Effect::ConfirmScreen` lists the screen once (after those questions, 1.2 s after the window is back) so ScreenCaptureKit's "bypass the system private window picker" alert comes up there, under a note saying to choose Allow (`View::picker_note`), rather than over the first recording.

Onboarding follows the system's light or dark appearance. The macOS window and its SwiftUI canvas resolve dynamic colors before the first visible frame and update together when appearance changes. Linux reads the desktop portal's color-scheme setting before opening the GPUI window, then refreshes the visible window when that setting changes; no user preference is written. Render every macOS screen and permission state with `apparent-helper onboarding --render DIR --light` or `--dark` (render-only overrides, not system preference changes).

Rows for draggable lists (`Permission::draggable`: Screen Recording, Accessibility) go straight to `OpenSettings`, which launches `apparent-helper permission-panel` (`capture/desktop/native/PermissionPanel.swift`) from the row's button: the card flies to just below System Settings (inside its empty lower area when the screen has no room; `PermissionPanel.dock`, checked by `capture/desktop/scripts/support/check-permission-panel-dock.sh`), drags the `Apparent.app` that contains the helper, reads grants from a fresh `--check` process (never trusts the drop), and is told `granted` when the row turns on. Preview it with `permission-panel --permission screen-recording --preview`; never `tccutil reset` Kit's real grants to test it.

### First-capture experiment

Enable `"experiments": {"onboardingFirstCapture": true}` before opening the tour
(restart the app or replay onboarding from Settings). It is off by default.
Existing users who already finished setup are not forced through it again.

On Shortcut, Continue stays disabled until a valid Record combination has been
entered in the inline recorder. Invalid, cancelled or reset choices do not count.
A verified desktop-owned Linux binding instead offers **Use desktop shortcut**;
Apparent cannot rewrite that compositor setting. The next page asks the user to
try giving feedback on onboarding: start with the chosen shortcut, talk and point,
then press it again to finish. This is a local practice capture, not feedback sent
to the team. Menu-started captures and cancelled or failed attempts do not finish
the tour. A successful bundle completion advances to Done.

Closing or choosing Try later is allowed, but leaves the experimental setup
unfinished so it resumes on the next launch. `onboarding.shortcutChosen` and
`onboarding.firstCaptureCompleted` persist those milestones beside `resume`;
`seen` becomes true only after a successful practice capture. The default flow
and permission-repair windows retain their existing behavior.

## Settings (`capture/desktop/settings/settings.rs`, `capture/desktop/native/Settings.swift`)

The shared config writer serializes CLI and GUI processes with a private
`config.json.lock` sidecar, held from before the file read until atomic publication.
Experiment edits include the nested map read in that transaction. Each writer
rereads the latest object, preserves unknown keys and secrets, writes a unique
`0600` temporary file, syncs it, then renames it inside a pinned canonical parent
directory. Config and lock symlinks are refused; the lock must be an owned private
regular file with one link. Parent directories must be trusted (a private ancestor
may protect otherwise group-writable descendants). The lock contains no preferences
and is not removed on unlock. Existing malformed/unreadable config still fails
closed. External editors do not acquire this advisory lock; normal setting read
timing is unchanged. Hosted operations acquire their queue lock before this lock,
never in reverse, and no network operation holds the config lock.

Native URL launch is a separate default-off experiment. See [native launch](native-launch.md)
for the exact `apparent://open` contract, explicit Linux handler registration and OS checks.

The onboarding pattern again. `apparent-helper settings` draws the `View` it's sent (General, Recording, Storage, Permissions, Agents, About; ⌘, or the menu's **Settings…**, and **Open Settings** on onboarding's last screen) and reports actions as JSON lines; `Panel` decides and writes `~/.config/apparent/config.json` through `handoff::set_config_value_in` (other keys and `0600` kept). If an existing config is malformed or unreadable, writes fail instead of replacing it with a new object and losing preferences.

Both shortcuts (`ShortcutKind::Record` ⌥⇧S, `Paste` ⌥⇧V) live under `"shortcuts"` (Carbon `keyCode`/`modifiers`/`label`; absent is the default, and `"paste": null` turns paste off; 0.3.x's file under Application Support is moved across once). Each shows only its keys, as rounded keycaps with a little depth in a recessed field, like Hex's control (`capture/desktop/native/ShortcutKeys.swift`). Holding its modifiers while Settings is in front sinks those keys onto their bases and turns the key red. Listening tints the field red: a dot breathes beside "Type a shortcut", and modifiers land as they go down (in macOS order) and lift out when let go, with a red caret after them. An accepted shortcut's keys bob once, left to right. A refused one turns red and shakes before the old keys return, with Rust's reason in the caption. Every value is a closed-form spring evaluated at a time (`Track`, `KeyController`), so the helper draws any moment the same live or rendered, and the TimelineView stops when the keys are still. Clicking a shortcut's row sends `record-shortcut`: Rust pauses every Apparent hotkey (`capture::pause_shortcuts`: Carbon unregistered, `emit_*` muted) so the current combo reaches the recorder, then `settings::validate` turns down combos without ⌘/⌥/⌃ (function keys excepted), ⌘ + one key, macOS's own (a static list plus the helper's enabled `CopySymbolicHotKeys`) and the other shortcut; `capture::set_shortcuts` registers both and puts the old ones back if `RegisterEventHotKey` refuses. Only a default answers to its aliases (⌃/⌘ added, ⌃⌥⌘, the ANSI key); a custom combo matches exactly in Carbon and the key event tap. Paste can be turned off (`ShortcutKind::can_unset`; Record can't, it's the way in) with the × beside its field or Delete while it listens (`clear-shortcut`, `Pressed::is_delete`): `ShortcutConfig::unset()` registers nothing and matches no key, and the field shows "Set shortcut".

On Omarchy, Hyprland's `code:201` is the Copilot key. At launch Apparent reads the exact `o.bind("SUPER + SHIFT + code:201", "Apparent: record", "…/apparent shortcut record")` definition from `$XDG_CONFIG_HOME/hypr/bindings.lua` (or `~/.config/hypr/bindings.lua`) and checks that a matching, uniquely described binding is live in `hyprctl binds -j` and its command is executable. This external system shortcut takes precedence over Apparent's saved record preference and default: Settings and Review show only Super+Shift+Copilot for Record, mark it as managed in Hyprland, and do not install a second Alt+Shift+S binding. Any old Apparent-described Alt+Shift+S binding in the live compositor is removed at launch. Change the binding in Hyprland, reload it, then restart Apparent. If the definition or live binding is missing or ambiguous, Apparent uses its saved shortcut or default instead; a description or an empty key in `hyprctl binds` alone is not proof of the Copilot key. An Apparent-owned Copilot shortcut can still be stored as `"shortcuts": {"record": {"keyCode": 201, "modifiers": 768, "label": "Super+Shift+Copilot"}}` (768 is Super + Shift in Apparent's stored modifier format).

Recording › Microphone (`capture/engine/runtime/audio/mic.rs`): `"microphone": {"id","name"}` holds an input id in `~/.config/apparent/config.json`; the name only labels a missing device. Absent = System default. Mac uses CoreAudio UIDs (also `AVCaptureDevice.uniqueID`). Linux with a PulseAudio-compatible server (including PipeWire) and `pactl` lists source names as `pulse:<name>`; `capture/desktop/linux/microphone.rs` opens the chosen source with GStreamer's `pulsesrc`, converting it to 16 kHz mono samples for the same capture buffer and Settings meter. A new capture waits for its first sample before accepting the source; a disconnected or unopenable choice falls back to the Pulse default shown in Settings, then to CPAL only if that default cannot open. It never changes the system default. Linux without a working Pulse server / `pactl` falls back to CPAL's available inputs; this may expose only the shared `default` or `pipewire` device, so distinct microphones cannot be selected there. Windows is unsupported; retained code is not a parity target. `mic::Watch` re-lists inputs while Settings is open: CoreAudio listeners on Mac, one-second checks elsewhere. `AudioRecorder` opens the chosen input on its worker thread, otherwise the current system default; a disconnected or unopenable choice remains saved but does not stop a new capture (`mic::Pick` explains the fallback in Settings and `note_pick` logs once). An error partway through a capture keeps samples heard so far. Mac's helper `MicMeter` uses AVCaptureSession; Linux GPUI Settings starts a separate input stream, stops it when leaving Recording, and expires stale level readings after 300 ms. The meter shows input samples, not recognized words or speaker identity; #16 owns no-transcript messaging. `apparent mic list|set <id|default>|watch` reports and changes the same saved selection.

Storage › Location (`CapturesDirView`): Change… opens the helper's folder picker and sends `set capturesDir` with an absolute path; Reset sends `""`, which removes the key. `handoff::default_captures_dir` reads it on every call (`APPARENT_CAPTURES_DIR` wins, then `"capturesDir"`, then `builtin_captures_dir`; `~` expands, a relative path is logged and skipped; tests never read the config), so new captures, retention, `apparent latest` and the size all follow at once. Captures already made aren't moved. A folder that can't be created or written falls back to the built-in one (logged once). State kept beside the captures (`inbox.json`, `last-mode`, legacy `config.json`) stays under `builtin_captures_dir`'s parent. The legacy config path stays under `builtin_captures_dir`.

Recording › Language shows under Transcription while it's Apple Speech: Automatic, then the helper's supported `locales` by name. It writes `appleLocale` (Automatic removes the key) and returns `Effect::SpeechModel`, which restarts the model check for the new language (see `capture/engine/docs/keyframes-and-text.md`).

The sidebar is the helper's alone: a row selects on mouse-down and its pane replaces the last on the next frame, with no transition (keep it that way; a fade read as lag; the level meter starts after the Recording pane has drawn). Rust's `state` never carries the pane (only `show` does, when Settings is opened again), so a reply can't move the selection.

About shows the package version and the running binary's build-time commit, for example `Version 0.3.13 · ad8d4b1`. Tracked local edits add `+modified`; builds without Git metadata or a valid CI revision say `build unknown`. This identifies the compiled build, not the selected update channel or an update staged for the next launch. Both the Mac helper and Linux GPUI Settings use the same label.

About has a Star on GitHub button below the update control. Its heading, action and caption share one box in the Mac helper and Linux GPUI Settings. It opens `https://github.com/tryapparent/apparent` in the browser; starring remains the user's choice on GitHub.

Settings follows system light or dark appearance without storing a preference. The Mac helper resolves the SwiftUI theme from its environment and the titlebar background from a dynamic AppKit color; an open window updates when macOS appearance changes. Linux reads the desktop portal before opening, then watches its color-scheme revision and repaints the open GPUI window. Hovered buttons and menu items use black on light lilac (`#c4a9e6`) in light mode, and cream on dark lilac (`#302442`) in dark mode; the selected sidebar always has cream text. The Linux update result panel also follows the palette, including its secondary actions.


Settings › About keeps its existing **Check for Updates…** action. On Linux, a manual check
shows Checking… only in the disabled check button and ignores duplicate clicks until it completes. Its dismissible result has
one line only: **Update available**, **No update available**, or **Update check failed**, with
the same pixel frog jumping, lounging, or puzzled on its notched lily pad. Failures and
unexpected worker responses are logged; they never become “No update available” or
appear as raw diagnostics in the modal.
The current Linux `check_cli` feed contract checks **and installs**: “Update available” also
covers an image already installed for the next launch. It does not mean a discovery-only check.
That result offers **Update now** (quit and launch the staged AppImage after exit) and
**Update later** (dismiss, leaving it for the next launch). The primary Update now action
uses brand Signal violet with Paper text; Update later uses the theme field and contrasting text and border.
Both use the brand's square geometry, 18px control type, and 44px hit height rather than
the compact pill buttons in Settings rows. The result panel has a square theme-colored field,
contrasting rule, 18px native status, and 8px-based spacing; failure text uses the semantic
failure color. Other results use a matching secondary OK action. Tab moves between
the choices, and Enter or Space activates the focused choice; Enter without a
focused choice chooses now. Escape defers. On other results Enter and Escape dismiss.
The app checks that an update is still staged before quitting; a failed relaunch request
shows the failure result instead of silently exiting. macOS continues to hand off to Sparkle's
native updater; it does not synthesize a frog result from a Sparkle launch.

Restart admission uses the shared coordinator's `capture_busy()` decision, including admitted
captures and detached finalizers whose capsule was dismissed. Settings/onboarding relaunches,
Linux Update now and Sparkle installation refuse while that decision is busy. Quit, SIGINT and
SIGTERM request normal Stop and wait for all finalizer receipts; a failed Stop leaves the app
open to show the error. The Mac AppKit termination guard cancels a busy native termination and
requests the same Stop path; its atomic boolean is only a copy of the coordinator decision.
Force quit/SIGKILL cannot finish a bundle: see narration-only recovery and durability limits in
`capture/engine/docs/capture.md`.

Before running `capture/desktop/scripts/packaging/install-app.sh` or `install-app-remote.sh`,
finish captures and quit the destination app. Both installers refuse replacement while an
executable inside that exact destination bundle is running, including its helpers; inability to
inspect processes also refuses. They do not kill other installations or CLI recorders, and
`APPARENT_NO_LAUNCH=1` does not bypass the replacement check. This is a pre-write snapshot,
not a lock that prevents a separate user from launching the destination during installation.

The three embedded illustrations in `desktop/assets/settings/` isolate the frog-and-pond scenes
from the selected concept. Each fits a transparent 256×192 device-pixel canvas; Settings
shows it at native device size, including fractional DPI. The rounded lily pads, water
ripples, coral cheeks, and the failed scene's broken cable and question mark remain.
The generated card background and labels are removed; result text stays native. Moss,
coral, and lavender in the artwork are **not new UI or status tokens**.


Launch at login writes `"launchAtLogin": true|false` to `config.json` before changing OS registration; if registration fails, it restores the previous preference. An explicit value is reconciled with `SMAppService.mainApp` (only inside Apparent.app) or Linux XDG autostart on app launch and every two seconds while running; stale Linux launch commands are refreshed when enabled, an absent value leaves existing OS registration alone, and a disabled/approval-needed OS entry is not forced back on. OS approval is still done in System Settings. There's no Dock icon: gpui 0.2.2 makes the app Regular after launch despite `LSUIElement`, so `ui::stay_out_of_dock` sets Accessory again at the top of `run_app`. The menu keeps Record, Choose What to Record…, Open Latest Capture, Export (flag), Settings…, Check for Updates…, Quit.

Agents › Command-line tool (`composition/command_line.rs`) links the app's own binary as `apparent` without asking for admin or replacing a different program. On Mac it symlinks the real app binary (which is how it finds the helper) into the first writable of `/opt/homebrew/bin`, `/usr/local/bin`, else `~/.local/bin`; Linux's installed AppImage provides a `~/.local/bin/apparent` link. A bare `apparent` from a terminal prints help instead of starting a second app. Agents › Agent skill shows `bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent` (`capture/agent/skills/apparent/SKILL.md`, also printed by `apparent skill`), which is the capture-reading skill. The separate `make-it-apparent` walkthrough is installed explicitly with `bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/make-it-apparent` (README › Agents); the default command does not guarantee its installation. This identity change does not migrate earlier captures, settings, OS permissions, or independently installed commands.

Agents › Ask for a Apparent video on complex changes toggles `"experiments": {"agentDemoVideo": true|false}`. Absent is on; set it to false to disable. Settings preserves other experiment flags and config keys; the next copy or paste reads the file without a restart. It adds a conditional request to the copied/pasted agent prompt, not the archived `summary.md`; automatic session delivery has been removed. The agent should only demonstrate a result it can actually see and report if recording is unavailable.

Render every pane (dark, light, each shortcut state: held, listening, partial, refused, accepted, custom; microphone chosen/missing/not allowed) with `apparent-helper settings --render DIR`, and one take on the shortcuts as 60 fps PNGs for a video with `--render DIR --shortcut-clip [--light]`; test with `cargo test settings::`.

## Lower other audio while recording (`capture/desktop/playback.rs`, `capture/engine/native/PlaybackVolume.swift`)

Off by default. General › Lower other audio while recording (`"lowerPlayback": true`) ramps the default output device's volume over one second to `playbackLevel` percent of what it was (Lower to, default 20%; 0 mutes) from the moment the shortcut is pressed or a recording starts, Mac or cabled iPhone, and back when it finishes, is cancelled or the picker closes without recording. `apparent-helper playback-volume --server`, started at launch while the setting is on, does the CoreAudio work and takes `start N` / `stop` lines on stdin. It reads every writable output channel's volume scalar before writing any (the master volume only when the channels have none), so balance and silent channels come back as they were; it never touches the mute flag or any input device. It restores when told to stop, when Apparent exits or crashes (stdin closes) and at once on SIGTERM, SIGINT or SIGHUP; it leaves the volume alone if someone changed it during the recording, and a device switched to mid-recording isn't lowered (the original device is restored). Fixed-volume outputs are left unchanged. Linux does nothing. `apparent-helper playback-volume --self-test` runs the ramp tests on fake devices; `--inspect` prints the current device's volume controls without changing them.

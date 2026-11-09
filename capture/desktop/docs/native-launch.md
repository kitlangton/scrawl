# Native app launch

## Hermes recording request

`apparent://hermes/<32-lowercase-hex-token>/<loopback-port>` opens the packaged
Electron confirmation for one recording. Ports are 32768–60999; paths, queries,
fragments and callback URLs are refused. The new route is separate from the
existing `nativeLaunch` experiment and never enables it or registers an OS
handler. The person must confirm **Record and send when finished** before the
bridge starts the installed app and uses its existing ID-bound capture CLI.
The native recorder retains permission/admission/resource enforcement.
On macOS the confirmation application hides before capture so it relinquishes
focus and does not become the recording's application context.
The URL token is a public request ID. A separate signing key lives in
`$XDG_RUNTIME_DIR/apparent-hermes/<token>.key` on Linux (default `/run/user/<uid>`),
or `$(getconf DARWIN_USER_TEMP_DIR)/apparent-hermes/<token>.key` on macOS.
The containing directory must belong to the user with mode 0700; the file is
created exclusively with mode 0600 and removed on normal helper exit. Hermes
reads it through the checked original local session, keeping the key out of
process arguments and persistent plugin state. Older pending handoffs require
manual paste rather than resuming the former public-token signing protocol.

Only the exact saved capture can provide `capture handoff <capture-id>`'s
reference. The loopback capability exposes signed status and that reference, never
capture controls, arbitrary files, credentials or transcripts. The Hermes
plugin sends through its own authenticated, captured connection/session; the
native helper holds neither session IDs nor gateway credentials. Cancelling
sends nothing. Closing the bridge leaves any admitted capture intact and ends
the return service. It expires after one hour. See the
[plugin routing and acceptance limits](../../../hermes/README.md).

Stock Hermes 0.23.0 blocks custom schemes in its external-link API. The plugin
uses its supported session-scoped safe-shell API for local conversations to
invoke the OS opener with this strictly validated URI. Remote and sandboxed
terminal refusals retain manual paste; the plugin changes no Hermes code or
OS registration.

Rollback is removal/disablement of the desktop plugin; manual paste is retained.
Remove the bridge after installed plugin consumers no longer use its URL or
`capture handoff` contract. Installed macOS/Linux launch, capture, permissions,
actual Hermes networking, model vision and receiver proof remain required.

## Open the idle app

The contract is the literal **`apparent://open`** on macOS and Linux. It opens the
idle app's existing onboarding window, or General Settings if onboarding is not
open. It never records, uploads, sends a capture, pastes, or changes a preference.
While a selection, recording or processing is active, the request is consumed
without raising a window or changing that work. This is deliberate: a website
must not put its launch window into a recording.

No trailing slash, credentials, port, additional path, query, fragment, whitespace,
escapes, alternate capitalization or other scheme is accepted. The URL carries
no data. Rejected URLs are not printed into app logs. CLI dispatch accepts one URL
argument, not a command embedded in a URL. No shell, local HTTP endpoint, SDK
permission change, localhost probe or installation-detection API is added.

## Enable and install

This feature is **on by default**. To disable URL launch, set
`"experiments": {"nativeLaunch": false}` in `~/.config/apparent/config.json`, preserving other keys.
The flag is read for each URL request, including requests sent to an already
running app; direct file edits take effect on the next request. Disabling the
flag rejects URLs but does not remove OS registration. Ordinary app launching
and an explicit `apparent launch` without a URL remain available without the flag.

Existing releases do not have this handler: update and restart the app first.
An older Linux GUI is not replaced or launched beside; quit it explicitly before
starting the updated one. A new same-user owner lock prevents simultaneous starts
from creating duplicate daemons. A bounded acknowledgement means the request was
queued, **not** that a browser can confirm installation or foreground success.

### macOS

The install, preview and release bundle builders declare `apparent` in
`CFBundleURLTypes`; GPUI's native URL-open callback applies the shared contract.
Install and launch via `./capture/desktop/scripts/packaging/install-app.sh`, never a background daemon from
`target/`. Use its normal Applications destination for URL verification: directly
opening a temporary staged bundle does not establish URL-handler registration.
On a safe Mac test surface, inspect the installed bundle with
`plutil -extract CFBundleURLTypes json -o - ~/Applications/Apparent.app/Contents/Info.plist`
and exercise `open 'apparent://open'` both cold and warm. LaunchServices chooses
the installed handler; multiple production/preview bundles may require explicitly
selecting the test bundle. A plist declaration alone is not native launch proof.

### Linux (desktop binary or AppImage)

AppImage packaging advertises `x-scheme-handler/apparent` and dispatches
`apparent launch %u`. Desktop integrations must retain that command, rather than
append a URL to the old bare `apparent` command.

For an installed AppImage or a stable executable path, explicitly run:

```sh
/path/to/Apparent.AppImage launch-handler install
/path/to/Apparent.AppImage launch-handler status
xdg-open 'apparent://open'
```

The install command requires URL launch to be enabled, writes
`$XDG_DATA_HOME/applications/apparent-launch.desktop` (normally
`~/.local/share/applications/`), asks `xdg-mime` to select it, then reads the
selection back. It uses `$APPIMAGE`, not an ephemeral AppImage mount path. It will
not replace a different existing desktop file. An executable path with desktop
escape/field-code characters is refused; move the app to a simple stable path.
Run the command again after moving an installation, inspecting/removing the old
owned `apparent-launch.desktop` first. The executable must have a simple ASCII
absolute path without spaces: generic `xdg-open` implementations cannot resolve
quoted executables. Unsupported paths fail before registration, not after a URL
has been clicked. Installation requires `desktop-file-utils` to refresh the MIME
cache used by browser portals. Status reports the desktop's selected ID;
it does not prove that its executable still exists or the GUI can start.

Installing the handler is an explicit OS preference change. **Opening a URL never
registers it or overwrites another handler.** To undo a test, restore the previous
`xdg-mime query default x-scheme-handler/apparent` selection and remove only the
test-owned desktop file. Remove a test's newly created MIME association too when
there was no previous handler. Never rewrite another user's MIME configuration.

## Website boundary and verification

Website buttons and fallback UX are separate work. A future integration can use
this exact URL from a user gesture. Browser confirmation may be accepted,
dismissed or left pending. Blur, visibility changes and timers are heuristics,
not evidence of installation or a successful open. No automatic download policy
is defined here.

Required native proof is separate on each OS: browser user gesture → actual OS
handler → cold app; repeat with a running app and confirm one GUI owner; reject
malformed URLs and flag-off requests; open during a real capture and confirm the
same capture continues. Inspect the saved bundle and movie. PR #162 records
Mac and isolated Linux proof of that contract. Default promotion preserves the
handler and OS-registration behavior; unit tests and packaging checks alone do not establish Mac parity.

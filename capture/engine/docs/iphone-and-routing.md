# Cabled iPhone capture and dev-site routing

Read this before changing `capture/engine/runtime/phone.rs`, `shared/routing.rs`, `capture/desktop/web.rs`, `capture/desktop/sdk.js` or the helper's phone capture.

## iPhone sources (`capture/engine/runtime/phone.rs`, `capture/native/ and desktop/native/`)

- `apparent-helper` (Swift, built by `build.rs` into `target/<profile>/`, copied and signed into `Apparent.app/Contents/MacOS` by `install-app.sh`) does SpeechAnalyzer, CoreMediaIO/AVFoundation phone capture with the mirror window, and movie decoding. Swift was chosen over objc2 bindings because SpeechAnalyzer has no Objective-C surface and `objc2-av-foundation` is not in the lockfile.
- Picker discovery (`apparent-helper devices`) reads the USB registry only: it is instant and never touches AVFoundation, so opening the picker can't raise a camera prompt or hang. The first time iPhone is chosen without Camera, Apparent closes the picker and opens the onboarding window's Camera row instead of starting the mirror (whose prompt would land under the picker).
- Live capture: the mirror window's rect (global top-left points, streamed by the helper) is the telemetry target, so ink/laser/clicks normalize into phone coordinates; inputs are re-based onto the movie's clock (`recording.startedAt`) at finish. Keyframes are selected on 320 px candidates and re-decoded from the native video at a 1600 px long edge.
- Audio for transcription is always the movie's audio exported on the asset timeline (`apparent-helper audio`). Do not feed a `.mov` straight to SpeechAnalyzer: `AVAudioFile` ignores the movie timeline and its word times drift (≈1.5 s late at 41 s on a phone recording).

## Dev-site routing (`shared/routing.rs`, `capture/desktop/web.rs`, `capture/desktop/sdk.js`)

Screen and Window captures are saved and copied, without looking up an OpenCode session or sending anywhere. Old routing code and `/sdk.js` remain for local dev-page registration, but nothing calls the capture-routing entry point. The app binds the browser SDK server to `127.0.0.1:47777`; it does not accept phone uploads.

The old `smartRouting` experiment has no effect on capture delivery.

# Native host client

`createNativeHostPort()` is privileged composition code for the fixed
`$HOME/.config/apparent/agent-control/capture.sock` endpoint. It returns only
the consumer-neutral `NativeHostPort` contract from `@apparent/app-contracts/host`;
pass this object to a Record-owned adapter in trusted composition, never expose
the socket or this client to a renderer. Example:

```ts
import { createNativeHostPort } from '@apparent/native-host';

const nativeHostPort = createNativeHostPort();
// Supply nativeHostPort to Record's owner adapter before composing RecordClient.
```

The protocol is v1, correlated by request ID, bound to owner `record`, limited to
five named operations, and uses the running GUI's `process.hrtime` deadline
timeline. The Rust listener checks same-UID peer credentials, private socket
ownership/mode, bounded frames, queue admission, and deadlines. Snapshot facts
are cached GUI observations scoped to that Desktop process. A screen-permission
read does not run a capture probe. A fresh GUI process has a new epoch; mutations
require its exact epoch and revision. Unknown mutation results are not retried.

Current operation limits:

- `readSnapshot` reports cached permission facts and the configured/effective
  shortcut projection. Uninitialized, paused, external, or unverified shortcut
  registration is reported as unknown/external and cannot be saved.
- `requestPermission` uses the app-owned permissions-only helper on macOS,
  independently of a legacy onboarding Session. An active legacy Session or
  busy helper refuses admission. Linux has no direct permission-request action.
  Dispatch acceptance does not grant permission.
- `openSystemSettings` uses that same helper on macOS. Linux supports only the
  Accessibility instructions route when a supported URL opener is available;
  other permission settings actions are unavailable.
- `validateRecordShortcut` uses the platform key-layout mapping, Desktop's
  reserved/taken checks, and the macOS native reservation query. It has no
  settings write or registration effect.
- `saveRecordShortcut` requires a fresh epoch/revision, an idle capture lifecycle,
  and proven live registration. It delegates registration to the existing
  runtime setter, compares persisted state inside the existing config writer
  transaction, reads the runtime value back, and rolls registration back on
  refusal where possible. Config compensation compares the value written by
  this request before restoring it, preserving a concurrent external edit.

This client does not implement safe relaunch, capture readiness, pen settings,
pickers, strokes, clipboard, or capture admission. It is not injected into an
installed route or Preview. Onboarding adoption must use
`adoptOnboardingProgress` from #731; host-held legacy journey-writer quiescence
is still a required, separately proven bootstrap lifecycle gate. Owner-envelope
presence does not prove quiescence, and this slice performs no adoption or
cutover. The existing repair UI/factory remains Onboarding-owned.

# Apparent Connect desktop client and remote CLI (candidate)

## Endpoint encryption candidate

Endpoint encryption is on by default for newly queued private Connect uploads and fetches. An explicit `experiments.hostedEncryption: false` in `~/.config/apparent/config.json` preserves the plaintext opt-out; file edits apply at the next operation, and changing the setting does not convert queued jobs or existing captures. Upload still requires its existing consent and account approval. Local bundles remain ordinary. Each endpoint uses the existing any-browser link/sign-in/approve journey for grant-bound public keys; an older approval without an endpoint identity needs a fresh approval. A missing approved reader leaves encrypted upload pending rather than sending plaintext. Account authorization is trusted; a compromised authorization service can substitute recipients. This is not Better Auth's token encryption or independent device pairing. See [the exact trust, wire, consent and remaining policy contract](hosted-encryption.md). Encrypted fetch requires the original private endpoint identity. Legacy plaintext is refused in encrypted mode and is not automatically migrated; an explicitly approved public copy remains service-readable. Updating source does not update installed clients or deploy the service.

Queued plaintext jobs retain their original format and do not automatically resume in encrypted mode after updating. To upload that local bundle encrypted, explicitly use `apparent connect upload <bundle>` after fresh approval; this creates a new ID. Existing hosted copies and pointers are not converted.

Shared macOS/Linux implementation, default-off `hostedSync` experiment. This is an implementation candidate, not a deployed service or proof of native GUI behavior. No Windows support. See [hosted-api.md](hosted-api.md) for the v1 protocol and hosting prerequisites.

## Implementation ownership

Desktop account controls appear by default in **Settings → General**. The
independent `connectSettings` flag supports an explicit false opt-out; close and
reopen Settings after editing it. Visibility does not grant upload consent. They use the same client device flow, private credentials,
renewal, queue cancellation and config writer as the CLI. There is no Connect
onboarding step. The Settings controller is owned by Desktop; the client's
secret-free presentation API supplies fresh local state, a validated public
verification URL/code and explicit bounded health checks, never raw responses.
See [Settings](../../capture/desktop/docs/onboarding-and-settings.md#apparent-connect-experiment)
for consent, cancellation, disable/disconnect and recovery behavior. This candidate
does not claim live account approval, deployment or platform proof from compilation.

`connect/client` is the production `apparent-connect-client` library, not a
protocol-only staging package. It owns transfer, device auth, credentials, queue
and worker locking, generation/cancellation checks, sharing identity/reservation/
revocation, secure staging, fetch publication and their real curl/loopback tests.
It builds independently with anyhow, Clap derive, libc, serde and serde_json,
plus the candidate's pinned age, ed25519-dalek and zeroize dependencies. It does not build the root app, GPUI, Engine,
GStreamer, Swift helper or root build script. Runtime curl and system SHA-256
requirements still apply; supported platforms remain macOS and Linux.

The account control receives only an optional bounded `accountInitial` from verified
email on device approval/renewal. Old credentials/server responses use a neutral
icon; full emails and credentials never enter renderer state. Fresh local readback
replaces or clears the hint on account change, disconnect or unusable state.

### Explicit public-copy admission

`Client::prepare_public_link` is the compatible client integration contract for
#171. Call it only after the sender explicitly chooses public sharing with:
“Anyone with this link can view this capture. Shared captures are not end-to-end
encrypted.” Existing Desktop callers still use `prepare_link`, which retains the
encrypted-sharing refusal until their consent/UI integration is accepted.

The explicit method persists a separate `publicCopy` job, plaintext manifest and
fresh capture ID under the current upload grant. It never edits/cancels the
private job, uploads endpoint identities, or discovers other captures. Repeated
requests/restart reuse that copy's ID while retained and eligible; cancellation,
expiry or a new credential generation requires a fresh explicit choice. Public
copies consume ordinary account quota and have the ordinary seven-day capture
retention. The existing capability grant controls anonymous reads and revocation;
the private bucket remains private. Revocation does not delete the private capture.

Public-copy intent is pinned across worker saves; workers still check current
upload consent, grant generation, cancellation and sharing availability. Private
format pinning is unchanged. Admission starts in `awaiting_share`; workers and
Retry cannot upload it until the foreground caller completes capability
reservation and commits `pending`. A crash/cancel earlier requires another
explicit public-sharing call. Local Paste pointers exclude public copies, and share
progress selects the public job. This API alone does not change installed sender
routing, enable sharing or constitute native/platform acceptance.

`connect/hosted.rs` is Desktop composition: it supplies `ConfigProvider` via the
accepted shared config store, browser opening, CLI/bundle-selector/logging
adapters, and the once-per-process worker launcher. All existing main/UI/Settings
callers use that facade and therefore the package. Remove a facade entry only
when those callers migrate to supported package composition; the `cloud` CLI
alias remains an external compatibility contract. Clipboard, feedback exclusion,
current-request cancellation and local fallback remain application policy.

`apparent connect shares list` now delegates pagination, UTF-8 cursor encoding,
share schema/ID/URL validation and final JSON assembly to the compiled
`typescript/dist/shares.js` policy through a bounded private stdio protocol. The
assembled CLI result is capped at 64 MiB; each authenticated page remains capped
at 2 MiB. Rust
still selects the read-then-upload credential and performs the fixed authenticated
page requests, token renewal and 30-second request timeout; it receives only
allowlisted `page` operations and returns response bodies over the private pipe.
The installed Electron run-as-Node runtime is resolved from the existing Settings
runtime. Its package must also stage `connect-client/dist/shares.js` beside that
runtime (`Resources/settings/app/connect-client` on macOS, `usr/share/apparent/settings/app/connect-client`
on Linux), including its ESM package marker. The Settings staging script builds
and copies that dependency-free module for both packagers. An installed CLI with
a missing module fails closed instead of using the retired Rust listing loop. No credential,
capability URL or response body is placed in arguments, environment variables or
logs. The root CLI and owner package remain Rust-dependent overall; this slice
does not claim Connect client independence.

`npm run test:cli` in `connect/client/typescript` runs the real CLI against a
synthetic local service; set `APPARENT_CONNECT_TEST_BINARY` to its absolute path.
The automatic Mac installed-app bridge lane also runs
`connect_shares_installed_electron_rust_roundtrip`, selecting the current CI
run/attempt app and requiring its bundled runtime and byte-identical policy.
This is installed CLI/child-runtime proof, not GUI or live-service acceptance.

Provider cloud/experiment reads are fresh at every check, including after client
construction and at prepared-link delivery. Root experiment reads retain
`handoff::experiment` overrides and test semantics. Cloud/experiment writes call
the real locked setters from #221; the package has no production config writer
or constructor snapshot. Lock order stays **worker → queue → config**. A provider
must not acquire the queue lock while publishing config. Provider write errors
propagate; they are not successful disable/login results.

Device login uses the provider's guarded cloud writer: cancellation is checked
again after the actual config lock wait and before atomic publication. The
application merges only the three owned cloud fields, preserving unknown nested
cloud keys, unrelated experiments and other secret-bearing config. Connect's
guarded writer wait is cancellable and bounded; other Settings writers retain
their existing policy. Settings queue waits are bounded without changing the
CLI/worker wait policy.

Disable/disconnect first publishes a private immutable `revocation-<id>.intent`
in the client's state directory, before waiting for the queue lock. Pending or
unreadable intents block new uploads, sharing, renewal and sign-in, including
after restart. A queue/config failure retains this fail-closed intent; retry
Disable uploads or Disconnect from Settings to finish the durable transaction.
Settlement removes only the intents covered by its committed effects, retaining
newer concurrent requests and honoring any pending disconnect. No network call
holds the queue/config locks. Do not manually delete an intent to recover.

Independent checks (run once through `t3-build` on Clanker):

```
cargo build --manifest-path connect/client/Cargo.toml --locked --offline
cargo test --manifest-path connect/client/Cargo.toml --locked --offline -- --test-threads=2
```

The root dev-dependency alone enables `test-support`: shared synthetic scenarios
run again with the real application provider and locked writers. Root retains
config races, exact subprocess/CLI alias and logging assertions, and the real UI
late-login helper. The feature is absent from normal application builds. Package checks do
not substitute for root adoption compilation or native/platform proof.

## Opt in on the recording computer

### Full beta and local recovery

The 20-place limit admits accounts, not simultaneous connections. Existing active accounts keep access when full. New Google identities are waitlisted and cannot approve upload/read grants; browser sign-in alone never enables uploads. The waitlist page explains that recording still works and offers local/authorized-transfer guidance.

`apparent connect alternatives` prints local inspection and complete-folder SSH/SCP instructions, consent boundaries and error recovery without contacting the service or changing configuration. SSH is optional; use an existing authorized route or ask for the intended destination/access. Older apps need an update for this command; `apparent inspect '/absolute/path/capture.apparent'` remains the local path.

Clipboard handoffs use Connect IDs only for current pending/ready private jobs. Failed, expired, cancelled and unknown states fall back to the original local path, with transfer guidance. Pending is not ready. Copy again after failure; already pasted IDs cannot be rewritten. On the recording computer, `apparent connect status` exposes the local bundle path and queue error. `beta_required` needs admitted account access, not another sign-in; network failures may be retried. The receiver cannot recover a local path from an opaque ID.

Connect's dependency-free `typescript/src/handoff.ts` owns eligibility and recovery text, executed through the existing packaged Node runtime and bounded private stdio. Rust retains consent/generation checks, queue state, process safety and file access; no persistence writer changes. A missing policy runtime falls back to the local pointer. The shared Settings staging script includes `dist/handoff.js` for both platform packagers. Rollback restores the old queue-pointer adapter; the legacy clipboard formatter remains for local image/transcript assembly until its owning workflow is migrated. This slice does not claim the entire client or Desktop is Rust-independent.

### Approve the recorder

Update Apparent to use `apparent connect`. Older apps with Connect support use
`apparent cloud`, which remains a compatibility alias in updated apps. The name
change does not move the API, credentials or `cloud` config keys.

```
apparent connect login --server https://api.example.com --scope upload --consent-upload
apparent connect status
apparent connect status --account
```

Replace the example with the real deployed API origin. The API and approval website can have different origins. Login prints the verification URI and one-time user code and opens the browser (use `--no-browser` to print only). Approve the requested **upload** scope on the website. This shares screen images, recording, voice, transcript, context and all supported bundle assets with the Connect account. `--consent-upload` is required before beginning upload authorization; successful upload login enables the experiment and the `cloud` preference. Read login never enables uploads. Tokens are neither printed nor copied.

Only newly completed ordinary GUI captures are automatically queued. Feedback captures are excluded. Enabling the feature does **not** scan or upload historical captures. An explicit historical upload is:

```
apparent connect upload /absolute/path/capture.apparent
apparent connect retry
apparent connect disable
apparent connect logout
```

`upload` explicitly queues that completed bundle **under a fresh ID on every invocation**, then makes one processing pass. It supersedes any unfinished local job for that bundle/server without changing an already reserved remote manifest. Use it to recover a changed bundle, an expired reservation or a tombstoned ID, or to deliberately share a previously cancelled capture. Automatic completion notifications deduplicate only retained queue records, including cancelled/expired records. Admission of another bundle prunes terminal or expired records; after pruning, a repeated completion notification for the original bundle can create a fresh ID. There is no permanent dedup history and no automatic historical scan. `retry` keeps existing IDs, resets eligible failed/pending jobs and makes one processing pass; it cannot enable disabled uploads or revive cancelled jobs. The running desktop worker resumes persisted pending jobs at startup, checks every two seconds, and retries with bounded exponential backoff (eight attempts, at most seven days, 100 queue records). CLI-only users run `retry` to resume after the command exits. `disable` acquires the queue lock before reading settings, clears consent/enabled state and cancels unfinished jobs durably under that same lock. `logout` additionally removes local read/upload transport credentials and renewal journals under the lock, then attempts whole-device-grant revocation with a shared two-second deadline outside all queue/config locks. An expired renewable access token uses the refresh credential without an expired bearer header. Offline or failed revocation does not undo local logout; a copied grant can remain usable remotely until revoked or expired. Neither operation removes independent decryption-key artifacts. Already sent server data is not deleted by local logout; it expires under the server's seven-day policy.

### Renewable native grants

The Better Auth candidate configures **3,600-second access tokens** and **2,592,000-second (30-day) refresh tokens**. The client uses the returned positive `expiresIn` and `refreshExpiresIn` seconds, not hardcoded lifetimes; it stores conservative Unix-second deadlines measured from before the request. Approved device polling and `POST /v1/token/refresh` must return `accessToken`, `refreshToken`, `scope`, `expiresIn`, `refreshExpiresIn` and `grantId`. Refresh sends only `{"refreshToken":...}`: it cannot request a scope or escalate a read grant. The returned scope must match, and `grantId` must remain stable. Server grant IDs are opaque and stored separately from the local 32-hex generation.

Every authenticated transfer, account read, share reservation/list/revocation and progress update uses the same renewal transport. It renews within 30 seconds of access expiry, or once after an unexpected HTTP 401, then retries the original request once. Network errors, HTTP 429 and 5xx retain credentials and allow bounded queue retry; permanent 401, expired refresh credentials, scope/grant mismatch or an invalid renewal response produce actionable `auth_required`. Ordinary renewal preserves local generation, IDs, manifests, spool contents, retry history and settings. Only explicit login creates a new local generation. Renewal never enables cloud gates, restores consent or revives cancelled jobs.

Local `status` does not make network requests. `readAuth` and `uploadAuth` expose only `status`, access/refresh deadlines and reapproval guidance: `signed_out`, `signed_in`, `renewal_due`, `legacy_reapproval_on_expiry` or `auth_required`. Tokens and grant IDs are absent. `--account` is the explicit network check and can renew. Legacy files without refresh metadata remain readable; after access expiry or server 401 they require one approval with `apparent connect login --server <origin> --scope read` (or `--scope upload --consent-upload`). Website sign-in alone cannot repair native credentials. An explicit replacement upload login invalidates old queue generations; use `apparent connect upload <bundle>` to deliberately recover historical work under the new grant.

One private per-scope advisory renewal lock serializes processes. It is never acquired while holding queue/config locks; the existing order remains **worker → queue → config**. No queue/config lock spans refresh network waits. A response is synced privately before publication; the journal records its completed state. Restart recovers a completed response without another rotation. Publication checks caller/job authorization, then rereads the exact credential and disable-cancellation epoch under the queue lock, so a late response cannot replace a new login/logout or cross a completed disable. Disable can let an in-flight rotation finish into its private journal, but cannot publish it for a cancelled job; a later independently authorized account operation can recover it without restoring uploads. The private `hosted/cancellation` marker is runtime cancellation state, not a preference or encryption key. A crash before a completed response is durably marked requires the server's bounded refresh-replay protection; after that window, reapproval can be necessary. Authentication does not establish encryption trust (#189).

Completion publishes a random 32-lowercase-hex ID to the disk queue before clipboard construction. Clipboard text starts with `apparent inspect <id>` (fetches to `./<id>.apparent`, waits up to 120 seconds, then prints the summary), even while upload is pending. A one-line README Agents link handles a missing CLI and leads to the maintained installation guide; image-inspection and the configured video-proof request precede the duration header and timestamped words/marks, which remain last for paste recognition. The prompt omits repeated login instructions, server lines and sender-local paths. The receiving CLI uses its configured server; configure a custom origin separately when required. Sign-in alone does not select this format: a retained, active private hosted job must exist. Without one, the prompt uses a shell-quoted local inspect path and local image/crop guidance. Archived summaries and bundle assets remain unchanged. It includes no bearer token. Hashing, media staging, network requests and retries run outside capture completion/paste. Network, auth, quota and upload errors leave the original capture and local clipboard handoff usable. Queue errors fall back to local-only handoff. `apparent connect status` is local JSON with queue path, status, attempts, next retry and sanitized error codes; `--account` explicitly queries account quota.

For scoped recovery, `apparent connect retry --id <capture-id>` resets and
processes only that retained pending/failed capture, preserving its ID and
leaving unrelated queued work unchanged. It cannot revive cancelled, expired,
ready or previous-sign-in jobs. Without `--id`, Retry retains its bulk behavior.

## Fetch on an agent's computer

```
apparent connect login --server https://api.example.com --scope read --no-browser
apparent fetch 0123456789abcdef0123456789abcdef --out ./capture.apparent --wait 120
```

A person approves a separate **read** device grant. Never paste an upload/read token into an agent prompt or silently expose one through the clipboard. Fetch does not require the upload experiment or consent. `--wait` bounds pending-manifest polling to 0–900 seconds; zero makes a single manifest request. Individual network operations are bounded separately (30 seconds for JSON, 60 for a file); the wait deadline does not cap transfer time once ready. The complete manifest and all files must arrive and pass size/SHA-256 checks before a sibling private staging directory is atomically renamed to `--out`. There is no lazy media retrieval. The parent directory must exist, and the destination must **not** exist: existing directories/files/symlinks are rejected, never merged or replaced. Traversal, absolute paths, invalid IDs, symlink paths, duplicate/unknown assets and invalid hashes are rejected. Failed fetches remove their staging directory and publish nothing.

After fetching, inspect `contact-sheet.png` visually and read `summary.md`/`apparent.json`, then use `apparent zoom` against the downloaded recording as usual. A pending/offline/expired capture is not evidence that the agent has seen the images.

### Local filesystem security boundary

Source bundles, their asset directories, output parents and hosted state must be in a trusted local directory tree: directories are owned by the current effective uid or root, with no group/other write access reachable by another uid. A private trusted ancestor can make a writable intermediate directory unreachable to other users. As a conservative rule, the source bundle, `frames/` and `frames-full/` directories themselves must never be group/other writable, even behind a private ancestor. The output parent must not be group/other writable unless it is sticky; sticky ancestors/output parents are accepted only with trusted owners and trusted owned children, so another uid cannot rename the private stage or trusted child. Sticky source asset directories are rejected because sticky does not prevent inserting entries. Nonsticky shared writable output parents are rejected before a stage is created. Symlinks are refused throughout the resolved tree. On macOS, extended ACL grants are also rejected (deny-only ACLs are accepted); mode bits alone do not describe Darwin ACL access. These checks never chmod or repair user directories. Choose an owned, unshared directory instead.

This boundary trusts root and the current uid, stable filesystem ownership/permissions and local POSIX/sticky semantics. It does not protect against hostile processes running as the same uid, root, or a filesystem/server that changes those semantics. Upload source aliases resolve to their canonical bundle first; bundled files and directories may not be aliases. The final source file is opened with `O_NOFOLLOW`; intermediate directories cannot be swapped by another uid inside the accepted tree. This is fail-closed tree validation, not a claim of universal race protection against same-uid writers.

## Inspectable preferences and secret artifacts

All application config publications use the shared `config.json.lock` advisory
lock, including ordinary Settings, onboarding, microphone, retention and Linux
update writes. It covers the whole read/modify/atomic-rename operation, not just
the hosted writers. Nested experiment edits read and update their map inside the
same transaction without reacquiring the lock. Thus a Settings save already in
progress completes before disable writes its false gates; it cannot publish an
old whole-config snapshot after disable returns. The sidecar is synchronization,
not another preference store: it stays in place and must not be deleted while
an application process is running. Lock order is **queue → config**, never the
reverse; neither lock spans device authorization or a network transfer.

Preferences live only in `~/.config/apparent/config.json` (`APPARENT_CONFIG` overrides this path):

```json
{"experiments":{"hostedSync":true},"cloud":{"enabled":true,"uploadConsent":true,"server":"https://api.example.com"}}
```

Preserve unrelated keys when editing. Login/disable use the existing atomic config writer. Login's settings snapshot, credential/grant-generation write and preference updates share the queue lock with disable/logout's settings snapshot, gate update, queue cancellation and credential removal. A login that commits before disable cannot restore stale consent after disable returns. Read login preserves both gates and cannot switch servers while uploads are enabled; a later explicit upload login can enable uploads with fresh consent. Device authorization, media staging and network transfers run outside the queue lock; active transports hold it only for brief state checks, so disable does not wait for a network timeout. Admission, retry and worker saves use the same lock and never acquire it recursively.

Direct file edits take effect on the next worker/handoff check, without restart; external editors do not participate in the queue or config writer locks. A gate observed off stops subsequent sends and cancels a transport already in progress at its next 20ms supervision check; bytes already transmitted cannot be recalled. A brief off/on config edit between checks is not a durable cancellation. Use `apparent connect disable` to cancel the queue durably rather than merely pause it through a config gate. Admission rechecks upload eligibility under the queue lock, so an enqueue paused before that lock cannot insert a job after `apparent connect disable` returns while uploads remain disabled. Under the same lock, the worker rereads the authoritative job before saving or allowing a transport to continue; a cancelled or superseded job cannot resurrect when gates turn back on, even with the retained credential. A new upload grant has a fresh generation; jobs tied to an old grant cannot send under it.

The `hosted/` directory beside `config.json` is private mode `0700`; `upload.credential` and `read.credential` are **secret artifacts**, not settings, written atomically with mode `0600`. Each contains server, scope, access/refresh credentials, access/refresh deadlines, server grant ID, reapproval state and local generation (legacy fields may be absent). Per-scope `.renewal` journals and `.response` files are also private secret artifacts, removed after publication or logout. Never remove an active `.renewal.lock`, `queue.lock` or `worker.lock` sidecar: changing its inode breaks process exclusion. Jobs (`<id>.job`, mode `0600`) contain local bundle path, ID, manifest, retry/status metadata and local generation, not tokens. Do not print or copy credentials, add them to a bundle, or put them in version control. Permissions/ownership/symlinks are checked before secret reads. Process crashes can leave private `request-*` scratch directories under `hosted/`, or `.apparent-fetch-*` sibling stages under the fetch output parent; remove abandoned scratch only after ensuring no hosted transfer is running. This client requires `curl` and the system SHA-256 tool (`sha256sum` on Linux, `/usr/bin/shasum` on macOS). Bearer tokens and device secrets go through curl stdin configuration/private JSON bodies, never subprocess argv. Curl defaults/netrc/redirects and stderr are disabled; only sanitized protocol errors are surfaced.

## Full fidelity and v1 limits

The allowlist exactly matches the API: summary, metadata, contact sheets including full-sheet originals, numbered `frames/` and `frames-full/` PNGs, recording MP4/MOV, WAV audio, `timeline.json` and `finish-trace.json`. Whiteboard content already becomes numbered frames plus manifest data; no separate whiteboard extension is needed. Unknown assets fail the entire upload with `unsupported_asset`, never silently disappear. Symlinks are not uploaded, even when named like an allowed file. Files are staged and hashed before reservation; retry requires the same manifest. Reordered server manifests are accepted only if their complete path/size/hash sets agree.

The server quota is **500,000,000 bytes per account**. Separately, v1 accepts at most 512 files and **90,000,000 bytes per file**, with a 128KiB manifest request. `file_too_large` is reported in queue status; the capture stays local. There is no multipart/chunk protocol and the client never drops a large recording/audio asset to fit. Real larger recordings therefore need a future coordinated protocol change. Server retention is seven days after reservation, with incomplete reservations reclaimed after 24 hours; a tombstoned ID requires a fresh explicit `apparent connect upload`, not a retry of that ID.

Local machinery is excluded before building either plaintext or encrypted
snapshots: Engine's empty `.publication.lock` and Desktop's `notes-ui.log`
(Chromium stderr). Both must be owned regular files with one link and no
group/other write permission. Neither is an allowed remote asset or evidence
reference; originals stay local. Unknown files still fail the upload. Known
asset failures retain their safe queue codes through encryption's snapshot
context rather than being misreported as an endpoint-key problem.

## Verification

`encrypted_settings_future_capture_restart_retry_and_disconnect` exercises one
continuous synthetic client lifecycle: Settings upload approval, repeated local
reads and an empty startup worker without history discovery or network requests,
a future capture interrupted during transfer, a new client retrying the same ID
and manifest, byte-identical complete-bundle fetch, and durable disable/disconnect.
A later upload approval cannot revive the cancelled future job. This uses the
production Rust client and curl against the loopback approval/storage fixture;
it is not live Google, deployed Worker, Electron routing or native platform proof.

`cargo test -j 3 --offline hosted_ -- --test-threads=2` exercises the production curl subprocess against an isolated loopback HTTP server, synthetic device auth, complete bundle upload/fetch, restart queue recovery, pending pointers, quota/disable/logout, pending/transport deadlines, corruption, path and symlink rejection and v1 bounds. Tests use private synthetic config/bundle directories, not the user's credentials/captures. Native capture/paste and macOS tooling still require platform proof; a Linux build is not GUI proof.

The independent package tests also exercise renewable account/upload/fetch/share/progress calls, proactive expiry and unexpected 401, separate-process single ownership, transient/permanent refresh failures, scope/grant mismatch, delayed refresh versus disable/logout/new generation, secret-free status, expired-grant revocation, offline logout and process exit after durable response before credential publication. These are synthetic protocol tests, not Better Auth/browser integration evidence; candidate backend-client integration and macOS/Linux native GUI checks remain separate gates.

### Recipient fetch before reservation

A pending pointer can precede upload reservation. Both hosted-ID `inspect` and
`fetch --wait 120` retry a privacy-preserving `not_found` within their existing
readiness budget, as well as a known pending manifest. `--wait 0` makes one
manifest attempt. A 404 does not distinguish absent, expired, deleted, wrong-account
or wrong-origin captures. Connect's TypeScript handoff policy supplies unavailable
and read-approval guidance; the existing `connect alternatives` recovery remains.
Authorization failures do not enter readiness polling. Integrity checks, private
destinations and atomic complete-bundle publication are unchanged. This restores
recipient behavior without evidence-first delivery or a claim about any incident's
exact cause.

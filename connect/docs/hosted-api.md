# Apparent Connect API (v1)

## Endpoint-encryption extension

The client makes this additive private transport available by default, with explicit opt-out and separate upload permission. Availability alone does not deploy a service or enroll an endpoint. See [the selected trust and versioned content contract](hosted-encryption.md), including its explicit departure from older independent-pairing assumptions. The account service is trusted to authorize recipients; it never needs capture content keys.

- `POST /v1/device/start` optionally includes `encryptionKey:{version:1,recipient:<canonical age X25519 public recipient>,signing:<64 lowercase hex Ed25519 public key>}`. The descriptor is immutable and becomes discoverable only through the same explicit account/scope approval. No key-registration bearer shortcut or new manual pairing ceremony is added.
- `GET /v1/encryption/peers` requires a read/upload bearer grant and returns `{accountId,grantId,scope,devices:[{grantId,scope,key:{version,recipient,signing}}]}` for approved, unrevoked same-account grants with live refresh authorization. More than 64 entries fails closed rather than returning an incomplete key list. Public keys are not secrets and are not proof of possession or independent pairing.
- Capture reservation accepts `format:'apparent-encrypted-v1'`. Missing format remains `plaintext-v1` for compatibility; unsupported values and mixed plaintext/opaque paths are rejected. Encrypted info/manifest responses repeat the discriminator. The existing immutable file manifest describes `envelope.json` and `sealed/part-NNNNNN.age` ciphertext transport, not inner asset names/hashes. Existing physical quotas, spending budgets, retention, per-file caps and cleanup still apply.
- Encrypted public/server-rendered sharing fails `encrypted_sharing_unsupported`, including capability reads. Clients must decrypt and validate locally before publishing an ordinary bundle. A format marker, successful upload or token renewal is not proof that decryption succeeded.

`0005.sql` adds the discriminator and grant-key/approval constraints without rewriting existing content or budgets. Do not execute it as a side effect of building/testing this candidate. There is no automatic plaintext migration, historical regrant, key escrow or production rollout in this extension.

Source availability alone is not deployed-service or live Google verification. The Worker owns API/auth only; Connect's canonical browser routes are `/connect` and `/connect/device`. The site retains `/hosted` and `/hosted/device` rendering aliases.

## Browser integration contract

Production topology: `SITE_ORIGIN=https://app.tryapparent.com`, `API_ORIGIN=https://hosted.tryapparent.com`, and optional transition `LEGACY_SITE_ORIGIN=https://hosted.tryapparent.com`. The API and registered Google callback stay on the existing hosted origin. Browser fetches use `credentials: 'include'` across these same-site origins. CORS and CSRF share one exact-origin allowlist: only SITE_ORIGIN and the explicitly configured LEGACY_SITE_ORIGIN are trusted. No wildcards, foreign origins, `null`, or missing Origin are accepted for preflight or cookie-authenticated mutations. Omit LEGACY_SITE_ORIGIN to disable old browser-origin access. Cookies remain Secure, HttpOnly, SameSite=Lax, Path=/, host-only. Cross-site deployments are not supported. The origin rename alone changes no schema/quota/scope; the separate Better Auth migration below deliberately requires one-time login/device reapproval for old credentials.

`GET /v1/session` always returns 200:

```json
{"authenticated":true,"csrfToken":"random session CSRF value","account":{"email":"verified@example.com","beta":"active","quotaBytes":500000000,"usedBytes":0,"waitlist":{"interestedAtFiveDollars":false}},"slots":{"used":1,"limit":20,"remaining":19}}
```

Anonymous shape: `{"authenticated":false,"csrfToken":null,"account":null,"slots":{"used":1,"limit":20,"remaining":19}}`. `account.beta` is `active` or `waitlist`. usedBytes includes pending reserved manifests that have not expired. Slots never exceed 20 and admission is atomic. Overflow identities can save their interest without billing.

Cookie-authenticated mutations require **`X-CSRF-Token`** from session plus an exact trusted `Origin` header. Missing/wrong values return `csrf_failed`. This applies to `POST /auth/logout`, `POST /v1/waitlist`, `POST /v1/device/approve`, and `POST /v1/device/reject`. Bearer endpoints do not accept cookies as authorization.

- `GET /auth/google/start?returnTo=/connect` redirects to Google with state, nonce and PKCE. Only fixed `/connect`, `/connect/device`, `/hosted` and `/hosted/device` paths (optional single `code` query, optional trailing slash) are permitted. Legacy paths normalize to `/connect` paths; arbitrary URLs, other query keys and fragments are rejected. Callback revalidates and normalizes already-stored legacy destinations too, preserving only the code.
- `GET /auth/google/callback` validates state cookie and stored one-time state, exchanges code, verifies signed ID token against configured provider JWKS, issuer, audience, expiry, nonce and verified email. Never accepts email supplied by the browser as identity.
- `POST /auth/logout` revokes the browser session, not separately approved device tokens; response `{ok:true}`.
- `POST /v1/waitlist` JSON `{interestedAtFiveDollars:boolean}` -> `{ok:true}`. Authentication required; no billing.
- `GET /v1/device?userCode=...` requires browser session; returns `{scope:'read'|'upload',status:'pending'|'approved'|'rejected'|'expired',expiresAt:'ISO UTC timestamp'}`. No device secret/token returned. UI must show scope and ask explicit approval.
- `POST /v1/device/approve` JSON `{userCode}` -> `{ok:true}`. Active beta required; binds the requested immutable scope to the approving account. No automatic approval on login.
- `POST /v1/device/reject` JSON `{userCode}` -> `{ok:true}`.

## Device and capture contract

Errors always JSON `{error:{code,message}}`; secrets/provider responses are never included. Authorization is `Bearer <opaque token>`, SHA-256 hashed at rest. Clients must not assume hex-only credentials or parse tokens. Read and upload authorizations are separate; upload also reads own captures. Better Auth's pinned OAuth Provider owns issuance/rotation; application grant checks enforce synchronous per-device revocation without relying on offline JWT verification.

`POST /v1/device/start` `{scope:'upload'|'read'}` -> `{deviceCode,userCode,verificationUri,expiresIn:600,interval:5}`. URI is SITE_ORIGIN `/connect/device?code=...`. `POST /v1/device/poll` `{deviceCode}` -> `{status:'pending'}`, `{status:'approved',accessToken,refreshToken,expiresIn,refreshExpiresIn,scope,grantId}`, or `{status:'expired'|'rejected'}`. Poll interval is enforced (429 `slow_down`). Approved poll retries return the same credentials until device expiry through a server-key encrypted envelope, with remaining lifetimes recalculated. Only hashes authorize bearer/renewal requests.

`POST /v1/token/refresh` JSON `{refreshToken}` -> `{accessToken,refreshToken,expiresIn,refreshExpiresIn,scope,grantId}`. Access lasts up to one hour; new refresh credentials last up to 30 days. Both lifetime fields are remaining **seconds**. Scope comes from the original approved grant, never a caller-supplied replacement. `grantId` is stable across every renewal. Atomically persist each new credential pair without changing the local authorization generation or re-enrolling decryption keys. Login/renewal do not grant capture decryption or sharing consent (#189).

Approved device-poll and refresh replies may also include `accountInitial`, derived
from the first character of that grant user's verified email. The email itself is
not returned. This optional display hint grants no authority; older replies and
credentials without it use a neutral account icon. Clients discard malformed
initials and withhold the hint when the relevant grant is unusable.

Duplicate refreshes can replay the same pair within the library's 30-second interval, including after Worker restart. While rotation is incomplete, return transient 503 `auth_unavailable` rather than invalidate a live concurrent winner. HTTP 429 `slow_down`, 5xx and transport failures are transient: keep the previous durable credentials and retry within bounded backoff. A **refresh** response of 401 `unauthorized` or `reauthorization_required` is permanent and needs explicit device reapproval. An expired access credential alone can still be refreshed. Reuse after the replay interval, expired renewal or an interrupted rotation that never persisted replay data tombstones only that device. D1 cannot make the library's separate rotation writes atomic; seamless recovery from every crash is not promised. Interrupted initial approval/issuance may also need a new device flow; see the service README's exact failure boundaries.

`POST /v1/token/revoke` with the current bearer revokes the **entire device authorization**, including outstanding access and every renewal credential, returning `{ok:true}`. Optional JSON `{refreshToken}` can revoke without a live bearer. If both credentials are supplied, they must belong to the same grant/account; a matching expired bearer is allowed, mismatched/unknown credentials return 401. Browser logout affects only its own session. Sibling read/upload devices remain valid. Durable authorization denial precedes fallible provider cleanup, so concurrent orphan token rows cannot regain access.

Capture IDs are lowercase 32-hex. `POST /v1/captures/:id` `{files:[{path,size,sha256}]}` atomically reserves the full manifest; identical order-independent retries succeed, differing manifests return `manifest_conflict`. Response `{id,status:'pending'|'ready',expiresAt}` (ISO UTC). Expiry is seven days from reservation; incomplete uploads expire after 24 hours to reclaim reservations. IDs remain tombstoned and cannot be reused after deletion/expiry. Quota 500,000,000 bytes, including pending. Maximum 512 files, JSON request 128 KiB, file 90,000,000 bytes (below Cloudflare Free/Pro 100MB request ceiling). Files larger than this are rejected at reservation with `file_too_large`; **no multipart extension is implemented in v1**. Clients must report this limitation, not split or silently omit assets. A full account can contain multiple bounded files; the 500MB quota is not a request-body limit.

Admission at exactly 500,000,000 live bytes does not evict. A valid new manifest that would exceed that account quota atomically tombstones the oldest live captures until it fits. Age means **server reservation order**, defined by D1's monotonically increasing `reservation_seq INTEGER PRIMARY KEY AUTOINCREMENT`, not the client ID, client capture timestamp, readiness, or request start time. This gives concurrent reservations and timestamp ties one deterministic order. Both ready and pending captures count and may be evicted; eviction removes an oldest prefix, never a newer capture to preserve an older one. Expired reservations do not count. The new capture insert, eviction and complete manifest-file inserts share one D1 batch transaction; a failed batch rolls all of them back. The schema change is for the new, undeployed `0001.sql`, not an upgrade of an existing database.

The complete manifest is validated **before any eviction**. A capture over 500,000,000 bytes fails with `quota_exceeded` (409), or a file over 90,000,000 bytes with `file_too_large` (400), leaving existing captures untouched. An existing ID never triggers eviction: identical retries retain the original expiry and order, conflicting manifests fail, and evicted/expired IDs return 404 rather than resurrecting. Under concurrent pressure a newly admitted capture may itself be evicted before its response is read, yielding 404; retrying that ID cannot evict newer captures. Evicted in-flight uploads cannot publish or complete. Tombstones deny reads immediately; scheduled R2 cleanup can lag and revisits late uploads, so the quota bounds live reservations, not instantaneous physical R2 usage.

Allowed paths: `apparent.json`, `notes.json`, `summary.md`, `contact-sheet.png`, `contact-sheet-full.png`, `audio.wav`, `recording.mp4`, `recording.mov`, `finish-trace.json`, `timeline.json`, `frames/frame-[0-9]+.png`, `frames-full/frame-[0-9]+.png`. This includes cropped originals and phone timeline data. Current whiteboards are appended as numbered frames plus manifest data, so these paths preserve them too (see `capture/desktop/ui/whiteboard.rs`). Max path 128 ASCII bytes, no dot/empty/absolute/traversal/backslash segments. Only regular client files; clients must reject symlinks before reading. Size is safe nonnegative integer; sha256 lowercase 64-hex. Zero-byte files permitted. Unknown assets must be surfaced by clients, not silently omitted. The exact local `notes-ui.log` diagnostic is excluded only when it is a current-user-owned, private regular file with one link. Empty validated `microphone-off` and `.publication.lock` markers also stay local. Other unknown or unsafe entries still fail closed.

`PUT /v1/captures/:id/files/<encoded segments>` sends raw bytes; streaming size bound and exact SHA256 verification precede publication. R2 verifies the supplied stream using its native `sha256` put option; completion checks its stored SHA256 checksum and exact size, never a client-asserted custom-metadata hash. Retry same bytes is safe: each retry validates its bytes through a unique temporary R2 key, then deletes that temporary object without overwriting the original object or mutating readiness. Pending objects are private and cannot be fetched. `POST /v1/captures/:id/complete` verifies every object and atomically transitions pending -> ready. Idempotent. `GET /v1/captures/:id` -> `{id,status,files,expiresAt}`. `GET /v1/captures/:id/files/<encoded segments>` returns private authenticated bytes only when ready and not expired. `DELETE /v1/captures/:id` upload scope only, idempotent -> `{ok:true}`; tombstones precede object cleanup. Owner mismatches return 404. `GET /v1/account` -> `{beta,quotaBytes,usedBytes}`.

Scheduled cleanup deletes tombstoned/expired R2 objects, retaining D1 tombstones. Authorization checks expiry synchronously, independent of cron. Upload attempts use unique R2 keys registered durably before writing; late/orphan objects are reclaimed by repeated cleanup, never readable after deletion. Each sweep selects at most 1,000 attempt keys in oldest-cleaned order, deletes in 100-key pages, then releases physical bytes and stamps each successfully deleted page in a two-statement JSON-table batch. Failed delete/stamp pages remain eligible; retained attempt records let later sweeps reclaim late writes. D1 conditional writes serialize quota, slot admission and finalization. No public R2 bucket or signed URLs.

Stable errors: `invalid_request`, `body_too_large`, `invalid_manifest`, `invalid_path`, `file_too_large`, `manifest_conflict`, `quota_exceeded`, `not_found`, `not_ready`, `size_mismatch`, `hash_mismatch`, `unauthorized`, `reauthorization_required`, `auth_unavailable`, `forbidden`, `csrf_failed`, `beta_required`, `device_unavailable`, `slow_down`, `oauth_failed`, `budget_exhausted`, `budget_unavailable`, `internal_error`.

### Evidence-first encrypted delivery

New private clients reserve the complete immutable transport manifest with
`format: 'apparent-encrypted-v1', delivery: 'evidence-first-v1'`. Each of `evidence/`
and `media/` contains `envelope.json` and one to sixteen contiguous
`sealed/part-NNNNNN.age` files. Mixed legacy/grouped paths, missing groups, unknown
delivery values and grouped paths without the explicit request field are rejected
before writes. The delivery discriminator is derived from the validated immutable
manifest, requiring no database migration. Legacy manifests and responses retain
their existing shape.

Grouped reservation, metadata and completion responses repeat `delivery` and add
`evidenceReady`. Evidence is ready only after every evidence file has a validated,
published storage object; individual uploads do not make a partial group readable.
Authenticated GETs of evidence paths may then proceed while full `status` remains
`pending`. Media and legacy reads still require full readiness; `/complete` still
verifies every file. Authorization, tombstones, revocation rechecks, aggregate
quotas, spending grants and cleanup remain in force. Evidence-first pending
captures retain the existing 24-hour incomplete deadline. Full readiness retains
the seven-day reservation expiry. Encrypted captures remain ineligible for public
sharing. No cloud decryption or processing is introduced.

### Initial beta R2 spending limits

The global D1 ledger permits at most **500,000 lifetime Class A** and **5,000,000 lifetime Class B** operation grants, with **no automatic calendar reset**. Every PUT consumes one A; every GET one B; pending completion atomically reserves the full manifest file count as B before any HEAD, even if it stops early. Operation grants are never refunded, including checksum failures, canceled streams and retries. Ready completion/metadata need no grant. Owner/auth/scope/path/expiry denial precedes the spending gate. Missing/invalid budget state or reservation DB error fails closed before R2 with `budget_unavailable` (503); exhausted valid budget returns `budget_exhausted` (503). A client must keep its complete durable upload job on either 503; no capture erasure, manifest shrink or omitted files. Caller input, token changes, restarts and deployments cannot reset the ledger.

Each PUT reserves a new unique attempt, one A operation and its exact file size atomically before streaming. The global **9,500,000,000-byte physical-attempt ceiling** includes live objects, in-flight streams, duplicate retries, orphans and tombstones awaiting deletion. Logical eviction/expiry does not release this counter. Thus 500MB/account is a maximum, not a promise of full simultaneous 20-account saturation; this cap can reject an upload despite remaining logical quota. Native R2 checksum/size verification and the 90MB/512-file fidelity contract remain unchanged.

Physical capacity is released once only after the unique PUT is known settled, a durable terminal marker exists **before** a successful delete, and the conditional release/stamp transaction commits. Failed deletion or stamp remains retryable. Uncertain/crashed grants stay charged even for absent/aged keys: a late PUT could recreate the key. Cleanup retains/rotates these attempts, never reuses permits, and needs no unbudgeted HEAD/LIST. DELETE/tombstoning and free R2 cleanup remain available when the spending caps are exhausted. There is no reset/admin API. Quiescent operator reconciliation is separate work, not an automatic timeout.

These limits bound only this Worker's controlled R2 operations/physical reservations, not Cloudflare's accountwide bill. Other credentials/resources, daily peak billing semantics and stored bytes during outages are outside this guard. The $1/$5 billing alerts are notifications, not caps. Free CPU, daily D1 rows/storage and real account configuration remain unverified.

## Explicit single-capture sharing (experiment)

The additive `0003.sql` migration creates capability rows; existing captures stay private. New grants require deployment variable `SHARING_ENABLED="true"` (default **false**) and the desktop sender is available by default unless `experiments.hostedSharing=false`. Turning the deployment switch off prevents new grants, not revocation or access to an already-issued link. No deployment is implied by this implementation.

- Upload-owner bearer `POST /v1/captures/:id/share` reserves or reuses the active capability immediately, even while upload is pending. Response `{url,status,expiresAt}`. Only the exact owned, live capture is eligible. Account login, ordinary upload and background processing do not call this route. Concurrent repeats return one active URL. Revocation then re-sharing creates a different URL; tombstoned/expired captures cannot be resurrected.
- Upload-owner bearer `POST /v1/captures/:id/share/progress` accepts `{state:'uploading'|'paused'|'failed'}`. Counts come from confirmed stored files, never caller byte claims. Ready capture state takes precedence.
- Owner bearer (read or upload), or an active browser session, `GET /v1/shares[?before=<cursor>]` returns `{shares:[{id,captureId,title,status,createdAt,expiresAt,url}],nextCursor}`. Pages contain at most 100 rows ordered by creation time and ID; continue until `nextCursor:null`. Cursors are `milliseconds:32hex-management-id`, not authority; account filtering still applies. Terminal rows have `url:null`. Title is currently the neutral “Apparent recording”, with recording/date/expiry shown separately.
- Upload-owner bearer or active session **with trusted Origin + session CSRF token**, `DELETE /v1/shares/:managementId` stops sharing without deleting private evidence. Read-only bearer cannot revoke. Management IDs are not public authority.
- Unauthenticated `GET|HEAD /s/:capability` renders useful escaped HTML: summary, time-aligned transcript, video, annotated keyframes and direct evidence links. `/manifest.json` exposes status, byte progress, expiry, evidence notice, checksums and direct asset URLs. `/files/<allowed path>` and `/thumbnail.png` return only that capture's ready evidence, with single `bytes=` ranges for media. Metadata/pending manifests require no R2 call. Each real asset GET and preview-text GET passes the same durable Class B spending gate as private reads; HEAD does no R2 work. Missing/exhausted budgets fail closed. No public R2 URL, signed storage URL or account credential is issued.

Capabilities are independent random 32-byte secrets (64 lowercase hex); only their SHA-256 hashes authorize reads. An authenticated owner can recover the same URL from a server-key encrypted envelope. Do not log URLs, secrets, authorization headers, request dumps or private evidence. Worker observability is disabled in the template; separately review any proxy/platform access logging before deployment. No third-party analytics or resources are loaded. All responses are `no-store`, `no-referrer`, `nosniff` and `noindex,nofollow,noarchive`; the HTML CSP allows only same-origin script/styles/media/images. The accompanying user request determines the task, including any request to carry out the sender’s transcript instructions. Incidental screen text remains untrusted evidence; existing permission and approval requirements still apply. Ordinary browser/file tools suffice; installation is not required. The optional secondary CTA is `https://app.tryapparent.com`.

Capability reads never authorize account listing, another capture, uploads, deletion or management. Every page/manifest/thumbnail/asset/range checks current revocation, expiry and capture tombstone, independently of cron; provider I/O is followed by a final authorization recheck before response publication. An already-started stream or third-party cached/downloaded copy cannot be recalled. Retention is unchanged: the existing recording expiry and incomplete-upload deadline apply, with no separate link duration. Terminal HTML exposes no recording evidence or previews; terminal assets/manifests return 410. Pending/paused/failed states do not claim readiness. Client polling is bounded to two minutes with at least five seconds between attempts.

Local tests include concurrent grant reuse, read-budget races across Worker instances, and revocation while HTML/manifest/thumbnail/range requests are blocked at storage/metadata I/O. These are local workerd/D1/R2 results, not production or provider-login proof.

## Deployment prerequisites

### Readiness evidence and action-scoped holds (#337 / #189)

Acceptance authority is [#337](https://github.com/tryapparent/apparent/issues/337),
[#189's accepted v1/default-on/public-copy limits](https://github.com/tryapparent/apparent/issues/189)
and [#180's deployment bounds](https://github.com/tryapparent/apparent/pull/180).
This is the owning readiness/hold map, not a second issue checklist. A hold applies
only to its named action. Continue source review, read-only investigation, local
synthetic tests, cutover/rollback preparation and reviewable candidate preparation
under the existing authority. Record what those establish and what they do not;
never turn a receipt, provisioned resource or anonymous HTTP response into live
acceptance. Missing access blocks that inspection, not unrelated preparation. Before declaring
access missing, check the existing credential-manager bootstrap/service references
and prior deployment handoffs, verify the appropriate user/account token endpoint,
and discover the actual bound service. Empty shell variables, a GitHub environment
404 or a template service name are not a credential/resource inventory.

| Acceptance boundary | Evidence and safe next preparation | Held action and authority; prerequisite to proceed |
| --- | --- | --- |
| Installed Settings, consent and private queue/fetch | #381 covers future completion admission, retry/cancellation and complete bundles in Rust. #384 provides installed Connect and no eager history scan. #390's installed Mac worker and isolated Linux fixture prove the same encrypted pending job resumes after restart, six-file hash equality, disabled restart offline and logout. These use synthetic approval and explicit staging, not real Google or a new capture/paste recording. Reuse only unchanged inputs; review finalizer/clipboard callers and obtain any missing changed native capture/handoff proof separately. Opening Settings must remain local-only. | No deployment hold blocks local review, fixtures or a focused native candidate. Native capture/paste evidence is not supplied by the worker fixture. A missing platform permission/runner must be identified specifically; preserve local-only onboarding practice and do not wait for #171's separate public-copy workflow. |
| Browser approval and OAuth/cookies | `service/test/access/oauth.test.mjs` exercises the actual Worker with a signed controlled provider, exact CORS/CSRF, scopes and cookie attributes; `web` browser tests use a labelled synthetic API. #337's first browser/native roundtrip injected a synthetic identity. A later private, independently reviewed Chromium/actual-Worker HTTPS fixture verifies real local Secure/HttpOnly/Lax cookie transport, signed controlled-provider redirects, scopes, CSRF, foreign-origin denial and logout without disabling TLS verification. #271 separately records authorized production Google/device approval and transfer cleanup. The later #430 static-instance cache retains live session/revocation reads and has real-D1 revocation coverage. The bounded deployed transfer below uses explicitly approved temporary account grants; it is not a new live Google GUI walkthrough. | Real Google audience/test-user/callback changes and account-access expansion remain held by #337/#180 and the owner deployment checklist. Actual acceptance needs the existing approved client/origins and an authorized test identity; local fixtures cannot supply that decision. |
| Authentication/encryption cutover | #187/#317 implement trusted-account enrollment, not malicious-authorization-service resistance. `renewal.test.mjs` and `encryption.test.mjs` rehearse incumbent metadata/spent-ledger preservation, retired credentials, additive format/grants and replay refusal in local D1. Source/crypto review and native worker proof do not establish remote schema or a professional cryptographic audit. Prepare backup/restore and ordered cutover below; retain the #189 accepted future-only/no-escrow/no-history-regrant limits. | Applying `0004` to an existing database retires old credentials and needs backup/stopped-writer approval (this checklist). #271 records its authorized application; confirm the live ledger before proposing it again. Applying `0005`, rewriting captures and activating other capabilities require their own exact action-scoped authorization; an approved private cutover does not authorize analytics `0006`/`0007` or public sharing. Need approval for the exact database, writer stop/cutover and verified backup/restore; no implicit migration or reseeding. |
| Remote bindings, Free viability and live end-to-end | The bounded cutover and live transfer below include actual version/binding/ledger/private-storage and provider CPU readback, targeted deletion/revocation and cleanup. They establish the measured beta workflow, not maximum-load capacity or later provider state. Read-only current-state inspection and focused synthetic preparation remain safe under existing access; do not read user capture contents. | The completed operation used an explicit, bounded existing-service release. It grants no continuing provider-write reservation: future provisioning, deployment, plan or ledger changes need their own [hosted activation](../../enablement/docs/continuous-delivery.md#hosted-activation-separate-operator-gate) authority naming resources, schema, test identity, limits and cleanup. Do not request a Paid upgrade as a substitute for Free proof. |
| Default private availability and opt-out | Settings visibility is already default-on; `hostedSync` and `hostedEncryption` are also available by default with explicit `false` opt-outs. Review the Rust `experiment_enabled_in` allowlist, client provider reads and config-preservation tests. The default/explicit-false, no-account/no-consent/no-history and legacy-job compatibility contracts must remain covered together with the feature map and owner docs. Keep `cloud.enabled` and `cloud.uploadConsent` false by default and independently required. | Default availability is not sign-in, endpoint approval, upload consent or a release receipt. Ship only after the named shared/native/encryption/service gates for changed boundaries are satisfied; preserve explicit opt-outs and unchanged consent. Exact-main verification still gates publication; source availability never authorizes unrelated activation or migration. |
| Public sharing, history, size and privacy | #171/#172 own explicit service-readable copies and any prior automatic-sharing opt-in; they do not authorize private endpoint-key disclosure. Keep their independently reviewed contracts. Do not grow #337 into recovery/history regrant, account expansion, #188 size expansion or outreach. | #189's public-sharing exception permits only separately consented selected copies; private encryption and sharing remain distinct. Their implementation is not a blanket blocker for private upload preparation. No schema application, bulk publication, Paid resource or real-user communication is authorized here. |

Completed bounded #337 acceptance: the explicitly authorized existing-beta cutover
applied only `0005` after backup and stopped-writer confirmation. Reviewed source
`8e27f399` subsequently supplied #430's static auth-initialization reuse, with live
session/revocation reads retained. A generated six-file encrypted transfer against
that deployed version passed approval, process-restart retry, scope denial and
decrypted hash equality; its test capture and temporary grants were cleaned up.
Bindings, secret types, routes, cron, quotas and private storage were preserved;
analytics remained unconfigured. This records that bounded operation, not current
provider state or authority for another deployment. Its exclusive provider-write
reservation was explicitly released. Any later operator must inspect current
state and obtain authority for their exact action; analytics and public sharing
remain separate scopes. The browser/native/source tests above are complementary
evidence, not a fresh live Google GUI or maximum-capacity claim.

Default-enabled installed worker proof additionally passed on Mac run
`41664119296` and isolated Linux at `a386b0f4`, including a real failed PUT,
same-job restart/retry, six decrypted hashes, disabled restart and owned cleanup.
These executions retain their original inputs; assess later changes under the
Desktop affected-seam policy rather than relabeling them as current binaries.

Evidence provenance: #390's original native run `41621191966` executed original
head `d57f881e`; its original landed tree `5e14301d` equals rewritten merge
`78d096bf` byte-for-byte. The rewrite changes commit identity, not that proof's
inputs. It is not a new run on later main. Keep private receipts, identities,
credentials and recordings outside public docs/issues; report only sanitized
scope/results and exact input applicability. Full CI runs only on main; focused
checks, independent review and applicable native proof precede merge. Exact-main
verification still gates publication.

A blocked handoff must state the affected row, last actual evidence, exact denied
operation and authority, completed safe prerequisites and the smallest missing
access/resource/decision. “Production/schema hold” alone is not a stopping reason.
Recheck this map and existing owner evidence before requesting access or repeating
work. Classify an anonymous probe using the actual native transport and response:
Cloudflare [error 1010](https://developers.cloudflare.com/support/troubleshooting/http-status-codes/cloudflare-1xxx-errors/error-1010/)
can reject a library-default User-Agent before the Worker. Compare an ordinary
native curl request and its CORS response; do not infer missing credentials or
backend failure from that probe, spoof credentials, or disable edge protection.
A successful anonymous response still does not prove authenticated/encrypted
acceptance. Keep actual resource identities and read-only inventory receipts
private. Issue #337 remains the acceptance tracker until the full outcome is met.

### Cutover rehearsal and operator packet

1. **Read-only inventory first:** record exact account/environment/Worker version,
   D1 migration ledger/schema and spent `r2_budget`, private Standard bucket/plan,
   route/CORS/Google callback and old writer/client compatibility. Use the actual
   existing service name, not `wrangler.toml`'s template name. Deployment config
   and readback require the same explicit `HOSTED_WORKER_NAME`; initial CD admission
   compares complete existing D1/R2/origin/secret metadata, route set and cron
   schedule before source ordering. A legacy
   non-SHA version tag remains an operator baseline decision, not an automatic
   ordering exception. Preserve API-only routes and existing variable/secret types.
   Record absent
   access as unknown, not empty resources, zero spend or successful readiness.
2. **Rehearse locally:** run the focused OAuth/renewal/encryption and Free-budget
   tests against the checked Worker. Generate deployment config with synthetic
   identifiers and run Wrangler `deploy --dry-run`; verify `nodejs_compat` is
   retained in the generated JSON as well as the template. Packaging success with
   a missing-runtime warning is not readiness. This never authorizes deploy.
   Authentication cutover from `0003` applies only separately approved `0004`.
   The local rehearsal also tests `0005` to prepare the encryption candidate;
   applying that additive encryption migration is a distinct held action, not
   part of authentication cutover. Do not replay `0001`–`0003`, reseed budgets or rewrite
   existing plaintext. Current `0006` is the separate analytics schema: do not
   use an unfiltered all-pending migration command to bundle it with `0005`. Prepare
   a hash-checked migration directory containing only the approved prefix through
   `0005`; the existing ledger must skip its already-applied entries. Legacy
   native/browser credentials require explicit reapproval.
3. **Prepare rollback before requesting cutover:** identify the previous Worker
   and a verified pre-cutover database backup/restore procedure, stop old writers,
   then apply only approved migrations through their ledger and verify preserved
   accounts/captures/shares/files/attempts/grants and spent counters. `0004` drops
   old auth tables: rolling back only the Worker is insufficient. On failure keep
   writers stopped and use the reviewed compatible version/database restore plan;
   never restore a stale budget over later writes or fabricate a down migration.
4. **Execute only after the specific approval/access exists:** verify same-site
   Secure/HttpOnly cookie transport, real Google audience/issuer/state/PKCE, separate
   read/upload consent, synthetic account isolation, encrypted complete-bundle
   transfer, restart/retry, disable/disconnect, deletion/cron and measured Free CPU/
   D1/R2 ceilings against that exact backend and changed native clients. Retain
   private receipts and delete only owned synthetic artifacts. Staging acceptance,
   production schema application, traffic activation and default shipment are
   separate decisions; none is silently bundled with this rehearsal.


**Local Free SQL budgets pass; remote acceptance is separate:** the real Miniflare audit in `connect/service/README.md` measures **5 SQL statements** for 512-file admission, **6 SQL + 512 R2 HEADs** for completion (518 combined internal operations), and **30 SQL + 10 R2 deletes** for a full 1,000-attempt cleanup (40 combined). Insufficient bulk B returns before HEAD with 5 SQL; ready completion uses 2 SQL and no R2. PUT first publication uses 7 SQL/1 R2; duplicate 9/2; checksum failure 8/2; denied grant 6/0; the tested lost-publication/404/catch branch 12/3. First-page cleanup delete/stamp failures use 2/4 SQL with one delete; tenth-page failures use 20/22 SQL with ten deletes. Each bounded page releases/stamps in two set statements, not SQL per object. The audit counts individual statements inside batches against Free's 50 queries and combined SQL/R2 against 1,000 internal-service operations; the 50 external-fetch budget is separate. Local 90MB streaming and these counts do not prove remote Free 10ms CPU, daily D1 rows/storage, scan cost or metadata-growth viability. No Paid activation is authorized. Parent must independently review and rerun native integration against the changed exact backend before publishing.

Workers Free documents a 10 ms CPU allowance with flexibility for infrequent
bursts, not an all-request percentile guarantee. Measure first-use, warm shared
auth and authenticated transfer/cleanup separately, and check actual
`exceededCpu`/exception outcomes. A successful bounded beta smoke does not prove
maximum-file, maximum-cardinality or high-concurrency capacity. Retain local
captures and queued recovery on resource failure, lifetime spending guards and
explicit account limits; it does not authorize a Paid upgrade. See the
[provider CPU contract](https://developers.cloudflare.com/workers/platform/limits/#cpu-time).

Use `connect/service/package.json` scripts. Preserve the workerd build condition and `nodejs_compat`. Bind `DB` (D1) and `BUCKET` (private Standard R2). Fresh databases apply `0001`–`0004` in order; before `0002` seeds the global ledger, stop writes and verify the exact bucket is fresh and empty. SQL cannot establish R2 emptiness. Existing data requires reviewed inventory/reconciliation, never reseeding. Existing deployments already through `0003` apply **only `0004`** after backup and stopped-writes review. This adds Better Auth/provider tables and grant authorization records, then removes the old credential/state tables. It changes no account IDs, captures, shares, quotas or budget counters. Old cookies become anonymous; pre-renewal tokens return 401 `reauthorization_required`, requiring one-time login/device reapproval. Same verified Google issuer/subject retains the incumbent account; never remap by email. Coordinate Worker/client cutover and rollback from a reviewed backup.

Configure API_ORIGIN, SITE_ORIGIN, GOOGLE_CLIENT_ID; secrets GOOGLE_CLIENT_SECRET, TOKEN_ENCRYPTION_KEY (32 random bytes base64), and separate high-entropy BETTER_AUTH_SECRET (at least 32 characters, no fallback). Register exact API_ORIGIN `/auth/google/callback` with Google. Discovery defaults to Google; `OAUTH_DISCOVERY_URL` is for controlled signed-provider tests only. Old individual provider endpoint overrides are retired; none are accepted from requests. Raw Better Auth registration, management, session and token handlers remain private. Login/renewal never enroll decryption keys. No real credentials are included. Parent must verify exact resource/account identity, initialized budgets, Free CPU/D1 limits, real same-site cookies, live Google and changed-backend native integration before separately authorized staging. No provisioning, plan/account change or deployment is implied.

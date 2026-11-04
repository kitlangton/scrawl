# Encrypted Connect handoff — issue #189 candidate

Endpoint encryption is available by default with private Connect; availability is not a completed security audit. Local `.apparent` bundles remain ordinary. An explicit `experiments.hostedEncryption=false` opts out. Encrypted transfer requires an endpoint enrolled through its existing explicit Connect approval; changing the flag does not enroll a key or grant upload consent. Availability alone does not grant credentials, change a deployment, upload history, migrate plaintext or guarantee key recovery.

## Accepted journey and trust boundary

Recorder and agent each click the supplied link, sign in if needed and approve their separate upload/read scope. Approval works from **any browser or device**. There is no manually entered pairing code, SSH prerequisite, key-file transfer, native-app launch or recording-computer ceremony. Keys are generated and kept at the endpoints. Normal recording, upload and fetch require no per-capture cryptographic action.

**Connect's account service is trusted to authorize encryption recipients and senders.** An actively compromised account/authorization service can add or substitute an attacker recipient/key. This design does **not** protect against that attack. A compromised storage provider without that authorization capability receives ciphertext, not content keys or capture plaintext. Compromised endpoints, agents with unrestricted access to their key files, and already-downloaded plaintext are outside this protection.

### Reconciliation with inherited #189 / #255 wording

Issue #189 originally required malicious-backend-resistant independent key enrollment. Design-only PR #255 (`e15b1efa`, only `docs/hosted-encryption.md`, 144 lines) proposed independent device pairing and predates the accepted journey above. Simon subsequently selected trusted account authorization and any-device approval. This contract explicitly supersedes that old enrollment guarantee; it does not claim to satisfy it by renaming server authorization as independent pairing. Better Auth's server-encrypted token replay envelopes are **not capture encryption**. The server-known approval code is never a content secret.

The old proposal has no production encryption callers to preserve. The live client is the independently packaged `connect/client/`, composed into Desktop/CLI through `connect/hosted.rs`; #253 is not a remaining implementation dependency. The merged Better Auth contract supplies immutable grant identity, explicit scopes, rotation and revocation. No donor implementation or duplicate #255 worker is introduced.

## Wire and endpoint boundary

The candidate uses binary age v1 with X25519 recipients. The maintained `age` library owns content-key creation/wrapping and authenticated streaming, including nonce/final-block handling. An independent Ed25519 endpoint key signs a domain-separated deterministic envelope containing version, API origin, account ID, capture ID, sender grant ID and the ordered ciphertext chunk manifest. This prevents a storage-only attacker from rewriting or swapping ciphertext under another capture/context. Sender public keys come from the trusted account service, not from the unsigned envelope.

All ordinary asset names, hashes, manifest fields, transcript, screen/audio/video bytes and summaries are inside one encrypted stream. The stream is split into bounded transport parts **after encryption**, not re-encrypted on retry. Public metadata includes API/account/capture/sender identity, encrypted format/version, ciphertext sizes/hashes, chunk order and signature; traffic timing and account authorization remain visible. Public paths are only `envelope.json` and `sealed/part-NNNNNN.age`. Existing per-file, file-count, physical-byte, account-quota and spending limits still apply to ciphertext. Encryption overhead can make a formerly limit-sized plaintext bundle too large; fail without dropping files.

The endpoint snapshots the completed bundle privately, encrypts/signs into an immutable private spool, and commits that spool before reserving a remote manifest. A retry or restart reuses identical ciphertext and the same capture ID. It does not generate new ciphertext under an existing reservation. Downloads check outer limits/hashes, sender authorization/signature, context and authenticated-stream finality; decrypted files are private scratch until every inner length/hash/reference passes. Publication is atomic and does not overwrite an existing destination. No failed, truncated or wrong-key download becomes a ready ordinary bundle.

Public-key descriptors are bound before the existing explicit approval; read and upload grants remain distinct. Renewal keeps the grant/key association. Active same-account approved grants alone appear in peer discovery. Missing private identity, missing approved sender, a revoked recipient or an unknown format fails closed. No server content keys, plaintext fallback, secret-bearing URLs, clipboard text, logs or command arguments are added. Keys are separate mode-0600 endpoint artifacts under existing private hosted state, not config values sent to the service.

## Consent, offline state and revocation

An approved reader while the recorder is offline means **approved—waiting for the recording computer**, not handoff ready. A recorder without an approved read recipient keeps the local capture pending. Once both endpoints are approved and the recorder runs, its existing background worker prepares the handoff. Browser approval does not imply a completed upload.

Admission pins the format. Toggling the experiment cannot turn a queued encrypted upload into plaintext or silently upload a previously queued plaintext job as encrypted. Disable cancels pending jobs; renewal cannot restore consent or revive cancellation. Read login never enables upload. Public/server-rendered sharing of encrypted captures is rejected, including existing capability read paths; no server decryption is introduced.

The separately authorized public-copy exception in #171 uses
`Client::prepare_public_link` after explicit non-E2E disclosure (see
[client admission](hosted-client.md#explicit-public-copy-admission)). It creates
an ordinary service-readable capture with its own ID rather than relabeling or
decrypting private server ciphertext. Private transfer, endpoint keys and the
service's encrypted-capture sharing refusal are unchanged. Existing Desktop
callers retain their refusal pending the consent/UI integration; this compatible
API is not evidence that the sender workflow or native gates are complete.

Discovery is rechecked at transfer boundaries and before plaintext publication. Revocation observed before a subsequent request stops remaining sends/publication. A request already in flight and ciphertext/plaintext/keys already obtained cannot be recalled. A spool addressed to a now-revoked recipient is not reused; it needs explicit fresh-ID upload from the local original. Revoking the sender also prevents new normal-client publication verified through that sender. There is no claim to erase authorized offline copies.

## Accepted v1 scope

Simon confirmed these limits on 2026-11-02:

1. **New-device history:** newly approved agents receive future freshly staged captures only. Existing ciphertext is not automatically rewrapped or reuploaded; historical regrant is outside v1.
2. **Lost keys:** losing every endpoint key makes existing ciphertext unrecoverable through password reset. No server backup/escrow or key-recovery UI is included.
3. **Existing plaintext:** leave existing hosted plaintext untouched and clearly identified. No plaintext data migration, deletion or reupload is needed. Encrypted-mode fetch refuses plaintext unless the operator deliberately leaves that mode to retrieve legacy data.

Default availability does not grant account access, upload consent or endpoint approval, and does not waive independent review and affected native/platform proof. The additive `0005.sql` schema change supports encrypted uploads; applying it is a separately authorized operational step, never a side effect of building/testing or migration of existing capture data.

## Verification and rollout gates

### Dependency provenance is not an audit claim

The candidate pins the supported maintenance release `age 0.11.5` (crate source revision `7f29934dc58e058e7a588ac3a9a7e946198a73a7` in `str4d/rage`) and `ed25519-dalek 3.0.0` (crate revision `07bef73ff85998a206cd2cea7f2605c801d0d1c9`). The age [v1 specification](https://age-encryption.org/v1) defines the stream/key-wrapping format; the integration uses the library's wrap/finish/decrypt APIs, not an age CLI subprocess. The [2019 Dalek review](https://blog.quarkslab.com/security-audit-of-dalek-libraries.html) mainly covered other Dalek libraries and only briefly examined Ed25519/X25519. It is not an audit of these pinned versions or this integration. No current-version independent cryptographic audit is claimed.

The first dependency scan found vulnerable `crypto-bigint 0.5.5` through the unused P256/HPKE functionality unconditionally compiled by age 0.12.1 (RUSTSEC-2026-0387). Rather than rely on a reachability exception, this candidate uses the co-released 0.11.5 maintenance line with the same parsing fixes and native X25519 stream API, removing P256/HPKE/crypto-bigint from both lockfiles. The updated introduced-dependency OSV scan queried 122 package/version pairs and still reports **two informational unmaintained build-time dependencies**, `basic-toml` (RUSTSEC-2026-0374) and `proc-macro-error2` (RUSTSEC-2026-0173), through localization configuration/diagnostic proc macros. They do not parse downloaded bundle data; they remain maintenance debt, not hidden exclusions or a zero-findings result. Independent integration security review and exact final-source verification remain separate required evidence.

Required: public enrollment/scopes/ownership/renewal/revocation; tamper, truncation, reordered/extra/missing parts, wrong keys, context substitution, randomized independent encryptions and age finalization; durable interruption/restart/consent races; byte-identical ordinary roundtrip with no plaintext in service objects; unsupported-format and encrypted-share denial; bounded resource failures; actual changed native worker/fetch wiring on macOS and Linux. A build or synthetic auth fixture is not GUI/live-Google proof. An agent review is independent code/security review, not a professional cryptographic audit.

Run normal reviewed hooks and exact CI on the final source. Do not apply `0005`, deploy, enable collection, grant production access or merge as part of this candidate. The parent batch orchestrator alone integrates main. Reconcile the tracked historical issue/PR criteria with this contract in the implementation PR rather than silently weakening their security claims.

## Evidence-first private delivery (#165)

New encrypted jobs pin `evidence-first-v1` delivery: summary, metadata, notes, timeline and original/annotated images are sealed separately from recording/audio. Both authenticated streams bind their group, capture, origin, account and sender. One immutable transport manifest reserves both groups under the same ID; retries reuse both ciphertext spools. Existing jobs and single-stream captures keep their original format. Older services reject the grouped paths; clients never downgrade encryption or silently omit media.

The worker uploads evidence before media. Authenticated metadata distinguishes `evidenceReady` from full `status: ready`; pending captures, including usable evidence, still expire after the existing 24-hour incomplete-upload deadline. Full completion, quotas, account authorization, revocation and cleanup retain their existing rules. Public readable copies keep complete-bundle behavior.

`apparent inspect <id>` downloads an immutable `./<id>.evidence.apparent` snapshot, validates every evidence file and prints its summary with a missing-media notice. `.evidence-only.json` explicitly identifies this local snapshot; the source metadata retains media references. It is not a complete bundle and must not be uploaded as one. Existing destinations are never overwritten. Inspect its absolute local path to reuse downloaded evidence. `apparent fetch <id> --media --out <new-directory> --wait 120` explicitly retrieves both groups, validates all references and atomically publishes an ordinary complete bundle. `--media` also documents the unchanged default full-fetch behavior. Use that complete directory for zoom/export or other media commands. No automatic fetch or in-place upgrade is performed. A pending/offline sender produces a bounded retry result; evidence readiness does not promise media availability.

No Desktop renderer, bridge, native capture, consent or browser playback route changes. Rollback keeps existing single-stream jobs readable; old clients reject grouped transport paths rather than publishing partial bundles. New grouped uploads require the compatible service to be published first.

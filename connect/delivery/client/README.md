# Connect TypeScript client components

This package supplies the adopted Electron account controller/presentation and
the crypto compatibility primitives for #529. It preserves the current Rust
`encryption.rs` wire rules:

- age v1 binary X25519 streams, concatenated after transport part splitting;
- archive bytes as a big-endian u32 descriptor length, ordered JSON descriptor, and exact file bytes;
- `apparent-encrypted-v1\0signed-envelope\0` followed by the signed JSON in Rust struct field order;
- strict Ed25519 verification (canonical encodings, weak public keys and small-order `R` rejected).

`validateRecipientEnvelope` is the parsed-DTO recipient boundary: it bounds the
envelope, enforces the v1 context and ordered multipart chunk rules, pins every
context field to caller supplied expectations, and verifies the signature with
the caller's trusted sender signing key. Its typed error categories contain no
wire content. Signature integrity is not service authorization: callers must
resolve the sender grant/key from current trusted authorization state first.
Use `validateRecipientEnvelopeBytes` for raw wire JSON. It enforces the 128 KiB
limit, valid UTF-8 and bounded JSON complexity, and rejects duplicate decoded
keys (including escaped aliases) before structural or signature validation.
The parsed-object API remains for already decoded DTOs; `JSON.parse` alone
cannot establish those raw-input properties.

`openRecipientArchive` consumes a raw envelope, ordered ciphertext parts,
endpoint identity and independently pinned context/sender key. It checks part
hashes, a bounded X25519 age header, authenticated decryption through final EOF,
the exact inner context, v1/group manifests, every file hash and bundle references.
No plaintext is returned until all checks pass. Ciphertext is consumed a part at
a time; validated plaintext (at most 500,000,000 bytes) is retained in memory.
Cancellation and errors return allowlisted categories, without wire content.
This API performs no service authorization, credential/config writes or filesystem
publication. Callers must resolve and recheck current sender authorization and
publish through the private, no-overwrite staging boundary. Identity persistence,
filesystem safety and production transfer lifecycle remain migration work.
Existing Rust callers remain authoritative; this module is not installed in a
production recipient path and does not replace the active Rust client.

## Authenticated recipient retrieval

`retrieveRecipientCapture` reads a capture through the real bearer-only HTTP
transport. The caller supplies a current read credential and endpoint identity.
It checks the authenticated peer registry against that local identity, resolves
the signed sender only from approved upload peers, pins account/origin/capture/
group independently, checks wire hashes before decryption, and rechecks current
sender authorization before returning verified in-memory files. Grouped
evidence-only retrieval skips media bytes. Legacy plaintext requires explicit
`allowLegacyPlaintext: true`; no automatic downgrade or token renewal occurs.

Requests never follow redirects, send cookies or include private data in error
messages. JSON/file byte limits and per-request deadlines cover response bodies;
`waitSeconds` bounds only readiness polling (0–900 seconds). Transfer requests
have 60-second deadlines individually, so callers should supply an AbortSignal
for an overall workflow deadline. Server bases retain the native client's HTTPS
or IPv4/localhost HTTP profile and exact root or `/connect` signed identity.

This API does not read or write credential files, refresh tokens, mutate config,
run a worker, or publish directories. Its caller remains responsible for the
single durable writer and private atomic no-overwrite publication; evidence-only
publication must retain the existing `.evidence-only.json` receipt. Installed
CLI/fetch paths still use the Rust implementation until those seams are adopted.

## Account Settings adoption

`settings-controller.cts` owns explicit consent, confirmation, asynchronous
request/receipt ownership, cancellation, retries, browser-action admission and
worker-activation intent. `settings-presentation.ts` owns the card presentation.
Desktop's existing `electron/connect.cjs` entry is generated from a thin TypeScript
composition entry plus these modules; prebuild/pretest compile them without Rust.
The installed Settings stager copies the same generated asset tree and existing
Electron runtime. No second runtime, service or config writer is introduced.

`settings-transport.cts` consumes the narrow backend RPC on the existing private
socket (`connect: backend`, `version: 1`; responses carry `account.version: 1`).
When the native peer advertises `deviceFlow: 1`, `device-flow.cts` owns the
installed device-start reply validation, public approval projection, polling,
expiry/rejection, credential validation and commit intent. The main process alone
receives private device replies; they never enter renderer views. Native code
executes the two allowlisted HTTP calls and the existing single-writer credential/
config transaction. It retains the approved response and original issue time;
commit consumes it once after fresh cancellation/marker/queue checks.

`settings-state.cts` derives local account state from `stateFacts: 1` private
metadata: configured origin, independent upload gates, grant usability/current
access, renewal/reauthorization and bounded account initial. It never probes or
mutates state. Raw facts and unknown metadata are stripped before renderer views;
malformed facts fail closed rather than using a legacy snapshot.

Native code still supplies safe private-record reads, renewal and durable mutations. Login pins the origin displayed at consent; activation needs
a successful matching operation and fresh committed upload gates. EOF, timeout
or feature disable before activation retains native durable cancellation. Failed
readback starts durable cleanup instead of leaving a reusable authorization.
These Rust HTTP/renewal/private-record/storage dependencies and older-consumer auth remain explicit migration debt;
this adoption does not complete the account or Rust-free stream acceptance.

Fallback to `legacy-settings.cts` is allowed only for an unsupported **read-only
opening**, before any action. It preserves the prior transport for older native
versions; its mechanically retained unchecked types are bounded compatibility
debt. No ambiguous mutation is retried. Remove it and the native controller only
after all supported consumers and native fallback surfaces migrate. The existing
native Settings path remains available for surfaces not yet replaced; no two
controllers drive one window.

## Local commands

Requires Node.js 22.18+ for the TypeScript test entrypoint.

```sh
npm ci
npm test
npm run typecheck
npm run build
npm run pack
npm run dev
```

These commands operate on TypeScript and npm dependencies; they do not invoke Cargo.
`dev` watches the owner package output; rebuild/relaunch the existing Desktop
frontend to consume a changed controller there, against an unchanged native
backend. `prepack` builds the exported runtime/declarations. `package-lock.json`
pins the direct and transitive npm graph.

The optional local service integration proof uses the actual built Worker with
isolated Miniflare D1/R2, real synthetic device approval/peer registration, signed
encrypted upload, retrieval on root and mounted origins, and revocation. From the
repository root, install/build the service before running it:

```sh
npm --prefix connect/service ci
npm --prefix connect/service run build
npm --prefix connect/delivery/client run test:service
```

This separate proof uses service development dependencies. It is outside the
independent client stream's default `typecheck`/`test` CI; those commands test the
bounded HTTP and recipient behavior without building the service or Rust.

## Dependency provenance and bridge evidence

- `age-encryption@0.3.1` is the upstream FiloSottile TypeScript age implementation: <https://github.com/FiloSottile/typage>. Its README documents Node.js 20+, binary `Uint8Array` streams and age CLI interoperability.
- `@noble/ed25519@3.2.0` supplies Ed25519. Verification explicitly uses the library's strict (`zip215: false`) mode and rejects small-order public keys and `R`, matching the current Rust caller's strict verification intent.

The reproducible `production_typescript_crypto_interop` Rust test calls the
actual production `pack` and `unpack` functions. It generates private synthetic
identities and a 33 MiB payload, then invokes `test/production-interop.ts`.
TypeScript verifies Rust's signed bytes/signature, stored identities, multipart
stream and original bytes, and publishes a newly encrypted/signed stream for
Rust to unpack. Both sides reject wrong recipients and tampering; strict
Ed25519 negative vectors are checked against Rust's `verify_strict` result.
All generated identities and payloads stay in the private test fixture and are
removed by its existing cleanup. No static private keys or large binaries are
checked in. Run this migration-only compatibility lane from the repository root:

```sh
APPARENT_CONNECT_INTEROP_NODE="$(command -v node)" \
APPARENT_CONNECT_INTEROP_SCRIPT="$PWD/connect/delivery/client/test/production-interop.ts" \
cargo test --manifest-path connect/client/Cargo.toml --offline \
  production_typescript_crypto_interop -- --ignored --nocapture
```

On Clanker, wrap that Cargo command once with `t3-build`, per root instructions.
This explicit cross-language migration check needs Rust; ordinary package
install/build/test/pack commands above do not. The fixture does not certify
production TypeScript transport, filesystem publication, account enrollment or
grouped evidence/media consumers, which remain migration work. The envelope
validator is exercised on the production Rust multipart envelope in this lane,
alongside synthetic context, bounds, signature and mutation cases. Authenticated
retrieval now supplies bounded decoding and current authorization-derived
context/key facts in TypeScript. Retain the active Rust caller and native
publication gates until installed consumers and safe publication are independently
verified.

## Canonical API base compatibility

Client settings and recipient contexts accept a legacy root API URL or the exact `/connect` API base. API routes are appended to that base; HTTP Origin/CSRF allowlists remain origin-only. The signed envelope context stays pinned to the exact normalized transport base, account, capture, sender grant and group. The migration-only production crypto lane exercises both root and mounted contexts through actual Rust pack/unpack and TypeScript encryption/signatures. Ordinary TypeScript package commands do not invoke Rust.

The active native caller remains authoritative migration debt. This source change does not update installed apps: publication/adoption of a client containing the mounted-base fix is required before the canonical setup command is available to those installations. Keep the existing root API compatibility profile until those consumers migrate.

## Queue and command adoption boundaries

The `./queue` export decodes legacy private queue records and proposes guarded
transitions. Callers must supply fresh facts under the authoritative writer lock,
including the captured cancellation epoch before applying upload results. These
functions do not read private files, renew credentials, commit proposals or start
a worker. Renewal-journal recovery and production state adoption remain with the
parent migration; Rust remains the sole active writer.

The `./recipient-command` export prepares headless fetch/remote-inspect behavior
and invokes the verified retrieval API through injected credential/publication
boundaries. Explicit local sources remain local. Successful results require a
committed publisher receipt; evidence snapshots require its evidence-only receipt.
That evidence-only requirement is pinned before invoking the publisher.
The command owns retrieved plaintext. Publisher callbacks borrow those buffers
until their promise settles; the command then wipes them on success, uncertainty,
failure or cancellation. Publishers must copy bytes needed beyond that lifetime.
This does not delete committed output or retained filesystem staging evidence.
This is not an installed CLI route or a filesystem publisher. A boolean receipt
from a fixture does not establish native publication. Actual credential loading,
renewal, private staging/no-overwrite publication and installed composition must
be adopted and verified before removing the Rust donor or enabling this path.

The `./credentials` export provides a private stored-wire decoder and pure renewal
reply merge. Its expected server must already have passed the caller's approved
origin boundary; matching that string does not approve a server. It preserves
legacy lifecycle tuples and pins renewal scope/grant and the original issue time.
Unknown fields are ignored as in the donor reader. A successful in-memory merge
is not a durable commit or permission to restore credentials after logout. The
parent's authoritative reader/writer must enforce private-file safety, renewal
journal ordering and fresh cancellation/generation checks before publication.
This API performs no file or HTTP access and must not be exposed to renderers or
used to serialize private values into CLI output or diagnostics.


The `./recipient-publication` export privately stages verified recipient captures
and publishes them with an injected, admitted native resource provider. It checks
ancestor/descriptor identity, ACL facts and private modes, synchronizes files and
directories, and retains uncertain publication without retry or deletion. The
provider must implement `checkAcl` and atomic no-replace publication; no fallback
is allowed. This library does not select the provider or switch installed callers.

The explicit native consumer suite is
`node --experimental-strip-types --test test/integration/publication-native.test.ts`.
Supply absolute `APPARENT_RESOURCE_SDK_CONSUMER` (isolated npm consumer with the
packed public `@apparent/native-resources` SDK installed) and
`APPARENT_RESOURCE_ARTIFACT` (qualified binary/manifest directory), plus
`APPARENT_RESOURCE_SOURCE_SHA` pinned to the reviewed producer. Admission must
verify producer provenance separately. This suite compiles no Rust, uses only
synthetic private fixtures, and needs real Linux and Mac execution before adoption.


`./credentials` also exposes `readStoredCredentialHandle`, a consuming private
file-descriptor reader. It bounds reads to 32,768 bytes, checks current-UID private
regular-file metadata and stability, requires native ACL acceptance, decodes the
strict stored wire format, wipes its byte buffer and returns redacted error
categories. Data hardlinks retain the donor reader's compatibility; lock files
still require their separate single-link rule.

The caller must admit the directory tree, open with NOFOLLOW, pin path/inode
identity, approve the server, and enforce fresh cancellation, revocation,
generation and expiry facts. A returned credential remains private stored facts;
reading it does not authorize use or clear `authRequired`. This reader performs
no credential/queue/config writes or renewal and does not switch installed CLI
routing. The legacy writer remains authoritative.


The `./state` export reads credential snapshots under the legacy `queue.lock`
and rechecks local login incarnation, cancellation and revocation before a
caller proceeds. It admits an existing private directory, pins directory/lock
inodes, and consumes the published native lock lease. The only filesystem
creation is the stable empty lock file; credential/config/queue JSON remains
owned by the Rust writer. Locks span short local reads, never network or renewal.

`readCredentialSnapshot` returns private stored facts, including `authRequired`.
`assertCredentialSnapshotCurrent` allows access/refresh rotation in the same
login, but refuses changed login identity, cancellation, pending revocation or
`authRequired`. Neither function renews credentials, checks token expiry or
establishes service authorization. The caller supplies an already approved
server and retains the transport/authorization and installed-adoption gates.


`./recipient-command/state` exposes `runRecipientCommandFromState`, composing the
real private state snapshot, legacy `<login-id>.identity` reader, authenticated
retrieval and native directory publisher. Identity files use the existing bounded
version1 X25519/Ed25519 format and are loaded only for encrypted manifests.
Missing/damaged keys are never replaced and never trigger plaintext fallback.
Legacy plaintext still needs explicit caller policy; local inspect touches no
state or resource. A changed local login/cancellation/revocation fence prevents
publication, and the command disposes its signing seed and plaintext buffers.

This is an actual command composition with injected admitted resources, not an
installed command/router. A completed legacy renewal journal with an exact stored
base may supply its saved response; expiries use the original issue time, and the
original stored incarnation remains the cancellation/revocation fence. Incomplete,
mismatched or malformed journals cannot authorize an orphan response. A matching
completed journal with a missing or invalid response fails closed. Recovery never
writes credentials, repairs markers or restores an upload gate.

After a retrieval 401, the command may reload writer state and retry once, only
with a different token in the same current login. It never sends a refresh request
or repeats the rejected token. Otherwise expired renewable access reports
`renewal_required`. Active renewal, provider assembly and installed adoption remain
separate gates. This path creates only the compatible empty lock file; Rust retains
all credential/queue/config JSON mutation.


A private host may explicitly supply `StateRecipientOptions.renewRead`, bound to
that same directory's **existing authoritative writer**. TypeScript still does
not write credential JSON. Each bounded GET prepares fresh same-login authority;
renewable access within 30 seconds of expiry asks the writer to renew. One 401
may force renewal and retry that GET once with a different token. A second 401
asks the writer to mark only the exact current read token `authRequired` and
stops. Cancellation, login, revocation and request deadlines remain fences.
Completed responses are committed by the owner before writer-backed transport
uses them. No upload/config/consent gate is restored.

`./recipient-command/renewal` exports the private `RecipientRenewalOwner` port
and `decodeRecipientRenewalReply`. The temporary legacy executable accepts
protocol-1 `renew_read` (server, generation, token, cancellation, force) and
`reject_read` (the same context without force). Replies contain only read-scope
`current` or `rejected`, or allowlisted failure categories. The decoder checks
exit status and the expected operation. Requests contain an access token: send
them only over the admitted private stdin pipe, never arguments, renderer IPC,
logs or shared temporary files. The host must bind the exact state namespace,
admit/protect the writer executable, propagate cancellation, bound its process
and output, and await its terminal result. Resource-helper admission alone does
not admit this legacy writer. The checked-in joined native test uses an
externally built, hash-pinned writer and isolated synthetic state; it is not an
installed locator or release. Without this explicit port, the previous read-only
recovery/reload behavior remains. Installed provider/launch assembly and caller
adoption are still pending. Remove this temporary legacy bridge when the
coordinated TypeScript writer and all its consumers pass the migration gates.

The internal `state/replacement` prerequisite stages a bounded private byte copy,
uses the admitted `replaceFile` operation under a caller-held queue lease, and
requires inode-pinned private readback before reporting commitment. It performs
no JSON policy or credential/journal ordering and has no production caller or
package export. It must not write the live namespace while Rust remains its
writer. After dispatch, uncertain outcomes retain observed files without retry;
proven no-effect results clean only the original stage inode under the original
admitted directory. Lost fences retain staging rather than delete unproven data.
Isolated Linux and Mac consumer tests cover replacement and stale-inode refusal
under the real queue lease. They do not establish installed acceptance; the
protected provider, coordinated sole-writer cutover and actual caller tests
remain required.

The internal `credentials/renewal-request` transport sends one bounded refresh
POST to the already approved root or `/connect` API base, with no bearer header,
cookies, redirects or retries. It returns raw reply bytes for durable response
publication before the existing journal merge; successful HTTP does not establish
valid renewed credentials. The caller owns journal preparation, locking, expiry,
currentness, recovery and wiping returned bytes. A failed POST may follow a
server-side rotation and never authorizes a blind retry. This has no production
caller and does not change the Rust writer or installed routing.

The internal `credentials/renewal-plan` module proposes renewal actions from
already decoded, fresh caller facts under the scope lease. It preserves the
legacy 30-second threshold, changed-token reuse and exact-base completed-response
precedence using the original issue time. It freezes detached credential/journal
snapshots and wipes its bounded response copy without changing caller buffers.
These are in-memory proposals; the caller still owns service approval, fresh
cancellation/revocation facts, durable journal ordering and sole-writer authority.

The native consumer fixture can exercise the protected installed wrapper with
`APPARENT_RESOURCE_PROVIDER=installed`. Alongside its admitted SDK, artifact and
source inputs, it requires `APPARENT_RESOURCE_MANIFEST_SHA256` from the trusted
producer return; it never derives that admission value from the supplied files.
This mode runs recipient/state/publication and replacement calls through the
public `./installed` factory, checks pending-call/lease retention, then retires
only the factory-owned snapshot. Missing or unsupported installed inputs fail
closed. The retained direct fixture is not an installed fallback. Linux x64
joined-consumer proof does not establish Mac joined proof or ARM admission;
installed routing and the sole-writer cutover remain separate gates.

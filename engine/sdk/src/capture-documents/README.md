# Detached capture documents (unpublished)

These helpers support the proposed `@apparent/engine-sdk/capture` contract.
This branch is a review candidate, not a frozen contract, native provider or
evidence of capture readiness. Capture-v1 and media/1 are unchanged.

The helpers verify exact detached UTF-8 bytes and their SHA256, envelope/payload
identity, clock mapping, collection references, stroke facts and publication
inventory inputs. Publication preflight requires the original detached bytes
and requested timestamps for each referenced derivative receipt. Decoder PTS is
independent of requested seek time, including start/end clamps.

These checks do not authenticate a producer, read filesystem contents, establish
native ownership or prove durability. Native publication still must validate the
complete existing ApparentBundle version-1 schema and all referenced/retained
media, enforce source/lease/publication authority, and perform no-replace durable
publication with crash recovery. Required role presence alone does not prove a
valid bundle. There is no new mandatory semantic companion file.

The capture/2 contract candidate now types and strictly validates source,
invocation, context/session/window/cursor, interactions, candidate, transcript,
and accepted-stroke facts. Context/session paths are absolute; window titles
remain manifest-only facts. Every observation is explicitly collected or
unavailable. Collection state and a typed sealed reference must agree: an
unavailable observation must be tagged unavailable. A successful observation may
legitimately collect zero windows/interactions or empty context; emptiness alone
does not prove that a native collector ran. Native producer proof remains required.
Source and invocation facts are checked against the same capture/materials
identity. This remains a review candidate; some native/platform schema and
authority gates are open, so it is not a frozen or available capability.

The package subpath `@apparent/engine-sdk/capture` defines a distinct
`channel: "capture"`, protocol version 2. It uses fresh 32-hex RPC IDs and
separate durable capture/material/publication identities. Requests reject
unknown fields, cap JSONL to 64 KiB, cap pending RPCs to 64 and queued writes to
1 MiB, and bound RPC timeouts to 60 seconds. Methods are `handshake`,
`discoverSources`, `start`, `inspect`, `stop`, `cancel`, `publish`,
`inspectPublication`, and `releaseMaterials`. Start names an advertised
display/area source and binds source identity, options, bundle identity and
invocation. No source fallback is allowed. The suggested launch command is
`<absolute executable> --runtime-directory DIR --capture-channel capture
--capture-protocol-version 2`; current `apparent-engine` does not implement
these flags and remains unavailable.

`materials-ready` means native media and detached materials are finalized,
synced and validated; it is never Saved. Publication requires exact plan bytes,
matching detached hashes and an ABSENT destination. Saved requires a native
receipt correlated to capture/material/publication/bundle/path/manifest,
inventory and plan hashes. SDK preflight checks the declared inventory only;
it does not establish a complete ApparentBundle, image/media validity, native
provenance, filesystem authority or durability. The required bundle profile is
the existing version-1 `apparent.json` plus `summary.md`, at least one frame and
a contact sheet, with existing conditional media references. No semantic.json
is added. Record persists the publication identity and exact plan binding in
private control storage before dispatch and never creates the final path.
Record may assemble the staged summary while materials are ready, before publish;
only copy/handoff readiness waits for authenticated matching Saved. Publication
observations, including failed/cancelled outcomes, carry the original publication
ID and plan hash. Reconnect inspection also requires the caller's restored
material/bundle/path/manifest-hash expectation and checks Saved fields against it.

Cancel has a capture or materials scope and does not release materials. A
materials-ready capture may be abandoned and released only through explicit
safe `releaseMaterials`; no timeout cleanup is defined. Publication identity
and plan hash remain stable across reconnect. A timeout does not cancel or
authorize a new allocation. Process close distinguishes observed exit from
signal requests; process death proves neither Saved nor material release.
Authenticated terminal inspection also returns the immutable materials manifest
while its source data is retained, so a reconnect can recover its lease and
explicitly release it even if the materials-ready reply was lost. After safe
release that reference is omitted. Cancel never silently releases sealed data.

The explicit `@apparent/engine-sdk/capture/testing` peer always handshakes as
test-double, unavailable, with capture capabilities false. It can exercise
bounded protocol failures and explicitly selected synthetic lifecycle
observations such as materials-ready or publishing; it never supplies native
facts or provenance. Those synthetic materials references are not native
artifacts and cannot produce a native Saved receipt. The separate capture-v1 and media/1 exports and semantics remain
unchanged. Native filesystem authority, full profile validation, durable
publication/recovery, platform proof and Record review remain gates. This is
not an availability claim or a contract freeze.

The candidate handshake shape is:

```json
{"channel":"capture","protocolVersion":2,"implementation":"native","readiness":"unavailable","capabilities":{"sourceDiscovery":false,"displayCapture":false,"areaCapture":false,"audioCapture":false,"contextFacts":false,"interactionFacts":false,"candidates":false,"transcript":false,"strokes":false,"publication":false,"retainedMaterials":false}}
```

Each request additionally carries a process `clientId` and a fresh `rpcId`:

```json
{"channel":"capture","protocolVersion":2,"clientId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","rpcId":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","method":"start","ownerToken":"<private 64-hex authority>","captureId":"cccccccccccccccccccccccccccccccc","bundleId":"take-1.apparent","bundlePath":"/captures/take-1.apparent","source":{"id":"display:7","generation":"dddddddddddddddddddddddddddddddd","kind":"display","displayId":7,"bounds":{"x":0,"y":0,"width":1920,"height":1080,"scale":1}},"options":{"audio":"none","localOnly":true},"invocation":{"kind":"explicit","invocationId":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","observedAtMs":0}}
```

The start source must have been returned by `discoverSources` for the same
provider generation. Source kinds are only `display` and `area`; a provider that
does not advertise them cannot be substituted with a screen fallback. A status
is one explicit `starting`, `recording`, `stopping`, `materials-ready`,
`publishing`, `saved`, `cancelled` or `failed` observation. `materials-ready`
carries the detached materials manifest. `saved` carries a native publication
receipt, never just a path or an EEXIST observation. Controls repeat the durable
capture ID and private owner authority. `releaseMaterials` additionally repeats
the exact materials and lease IDs.

Open review points remain: exact compatibility of context/session and source
facts with Record's accepted handoff schema; native capability names and their
provider-generation rules; the authenticated filesystem authority behind
detached identities; complete bounded `apparent.json` and media validation;
the pinned same-filesystem, no-replace publication journal and crash
reconciliation; cancellation versus the atomic publication commit; and the
native macOS/Linux proof. These are not silently decided by SDK shape choices.

## Native enforcement required before availability

The native owner must authenticate capture authority before receipt recovery or
reading caller-supplied plan paths. It durably binds owner, capture, bundle/final
path, source generation, options and invocation before acquiring native capture
resources. Same-identity replay recovers that operation; a changed binding fails
without starting another capture. Current permission/source readiness must not
prevent authenticated inspection or control of an already-owned operation.
Source generations must change when their native identity/consent is invalidated;
an advertised source cannot silently become another output. First-frame readiness
requires actual selected-source buffers, not admission or a reference fixture.

Sealed materials and derivative/inference receipts are resolved against the
Engine-owned durable registry, with pinned native file identities and exact-byte
digests. A caller-provided hash or path alone establishes no native provenance.
Explicit material release requires matching owner/key/lease and exclusion of
active native readers, capture and publication; a failed or lost release answer
does not establish deletion. Media jobs do not take live-capture admission.

Publication first durably binds the immutable publication ID and exact plan hash.
It snapshots pinned inputs into an exclusively owned sibling under the pinned
destination parent, validates the complete bundle/media and exact inventory,
and syncs files/directories. Final visibility requires same-filesystem atomic
no-replace rename (Linux `renameat2(RENAME_NOREPLACE)`, macOS
`renameatx_np(RENAME_EXCL)`), followed by parent durability and a durable receipt.
An existing destination is preserved; it is never overwritten or taken as Saved
from EEXIST. No weaker rename fallback is allowed. The native journal must retain
the staged directory identity, plan and inventory digests before commit so a crash
between rename and receipt can reconcile the exact same artifact, not a path scan.
Cancel and the commit decision serialize under native publication authority:
before commit cancellation wins; after commit the result is Saved. Ambiguous
durability stays unresolved until reconciliation proves the result. These are
implementation and platform-proof requirements, not claims this SDK supplies them.

Record's committed processing baseline is PR715 (`495715bf`): its TypeScript
formatter consumes target/session/context, transcript, selected-frame, note/mark
and warning facts and returns `summaryMarkdown` only. It owns their translation
and semantic choices. Its `"local-clipboard"` presentation is used by the newer
pure `formatLocalHandoff` wrapper; that wrapper takes caller-attested Saved path
and semantic input, and performs no publication or clipboard effect. The
formatter remains the consumer boundary for the fact schemas here. No canonical
manifest writer or authenticated publication consumer is supplied here. A new
capture-channel allocator must persist its
identity/plan binding in a separate private control namespace and leave the final
destination absent. The existing v1 reservation-directory allocator is incompatible.

Connect producer prerequisite

connect-delivery.yml builds and verifies browser and Worker outputs on main push
or manual main dispatch, without native CI. It uploads separate immutable Actions
artifacts named connect-browser-<full SHA> and connect-worker-<full SHA>, each
containing manifest.json and dist/. The reusable publisher accepts required string
input `producer-run`; an empty input selects the producer, a nonempty input selects
publication. Public invokes it independently with `secrets: inherit`.

Run `node --experimental-strip-types connect/delivery/artifacts.ts seal DIR KIND`
then `validate DIR KIND` from a clean committed checkout. KIND is connect-browser
or connect-worker. The manifest records source identity, input and output hashes,
production environment and runtime versions. This is local integrity evidence,
not independent trusted GitHub-run admission; Public owns that integration.

Producer analytics covers shared configuration, production artifacts and synthetic
Connect consent behavior. Public owns composed analytics transport acceptance.
Five browser assets still come from Public. Existing
Python theme checks and native Connect client/consumer coupling remain; this
producer is not complete Connect language or delivery independence.

Worker publisher transition

The publish job in connect-delivery.yml consumes authenticated canonical producer metadata and verifies
its GitHub archive SHA256 before extracting exactly manifest.json and dist/worker.js.
The artifacts CLI validates complete candidate inventory, output hashes and source
ancestry against current main. Wrangler receives the verified payload with
--no-bundle; no product build runs in the publisher.

Publication retains HOSTED_CD_ENABLED, hosted-production bindings and the common
web-delivery-production-eligible lock. The retirement check deliberately
fails while web-delivery.yml still declares its hosted job. Public must remove that
job atomically with adoption; this is an existing-publisher retirement contract.
Browser composition remains Public-owned. Native consumers still couple to Connect;
this Worker publisher does not establish whole-product independence.

runtime/ contains byte-identical transitional copies of hosted_config.py and
verify_worker.py. PYTHONPATH supplies the existing enablement/release_gate import;
the originals remain untouched. Remove donors only with Public's coordinated
consumer migration; move release_gate ancestry policy only as a separately verified
compatible owner change. Runtime controls, resources and migrations are unchanged.

Selection enumerates authenticated successful main workflow runs in pages of 100
until an empty page, checking direct browser and Worker artifacts against complete
current inputs and production environment. Receipt-only runs do not truncate the
search. Manual dispatch forces a fresh producer run.

The callable requires string input `producer-run`: empty runs selection/production;
nonempty publishes that successful run. Each completed invocation uploads
`connect-selection-<SHA>` containing exactly `connect-selection.json`, with JSON
`{"schema":1,"selectedRun":""}` for a fresh build or a prior direct producer run ID
as the string `selectedRun`. Publication authenticates the current receipt and its
one-hop prior producer, including archive digests, expiry and Git ancestry. Outputs
are `selected-run`, `build`, and `applicable` (currently always true).

Recipient producer prerequisite

`connect-delivery.yml` runs only on canonical main and builds the standalone
Rust recipient in a temporary source directory, outside root Cargo configuration.
Each of four macOS/Linux architecture outputs includes its executable and a
protocol-1 manifest sealing the actual executable SHA256, full source SHA and
unique workflow run/attempt version. GitHub build attestations cover both files.
A main-only release job verifies both attestations against the canonical workflow,
main ref and exact source digest, then publishes four target archives and SHA256SUMS
as durable GitHub release assets. Versions are independently named
`recipient-<full SHA>-<run ID>-<attempt>`; reruns create a new version. Existing
versions/assets must never be edited, deleted or overwritten. Release creation
fails on an existing tag; no update/upload-clobber path exists. Actions archives
are only intermediate transport. GitHub repository administrators can still alter
releases: consumers must verify the embedded executable and manifest attestations,
not trust a release URL or checksum alone. These binaries are not Apple signed or
notarized. No Desktop packaging, app release or hosted activation is included.

Before adoption, authenticate the canonical workflow run on main and its source,
verify GitHub attestations with the expected repository/workflow identity, validate
platform/architecture/protocol and hash the downloaded executable before copying
it into the application. Never fall back to source compilation. App signing,
notarization, updater identity and exact-commit publication gates remain required.

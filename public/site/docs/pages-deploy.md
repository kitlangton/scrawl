# Capture website delivery on Cloudflare Pages

The canonical repository is `https://github.com/tryapparent/apparent`, branch
`main`. `.github/workflows/value-stream-delivery.yml` is the sole production publisher for
`app.tryapparent.com`. It publishes the isolated **apparent-capture** Pages project
in account `a54dcebead3b8ad688f891da0ab54d1f`. It does not publish the document-product
project `tryapparent-website`, the hosted API Worker, or `public/site/wrangler.jsonc`.

## Public artifact and accepted design

Use Bun 1.3.10 and Node 22.23.0. Connect publishes verified immutable browser
artifacts separately; Public installs only its own dependencies. From the
repository root, with an Actions-read token and the approved Public build variables:

```sh
(cd public/site && bun install --frozen-lockfile)
node --experimental-strip-types public/site/scripts/cd/stream-artifact.ts .build/public-artifact
node --experimental-strip-types public/site/scripts/connect/artifact.ts .build/connect-browser
export CONNECT_BROWSER_ARTIFACT="$PWD/.build/connect-browser/dist"
cd public/site
cp -a ../../.build/public-artifact dist-apparent
node scripts/connect/compose-connect.ts
node scripts/pages/prepare-pages.ts
COMPOSE_PUBLIC_INPUT="$PWD/../../.build/public-artifact" node scripts/site-build/verify-composed.ts
node scripts/pages/prepare-pages-video.ts
PAGES_WORKER=dist-apparent/_worker.js node --test scripts/pages/test-pages-video-runtime.ts
node scripts/cd/publish-pages.ts seal <full-commit-sha>
```

Upload only the sealed `dist-apparent/` using the publisher. Never upload `dist`, this repository, the artifact manifest, evidence, private prototypes, pitches, internal feature maps or
operator metadata. `404.html` disables Pages' implicit SPA fallback. The preparation
step checks every input path against the former Caddy public boundary, including
only the three approved hosted illustrations. `_headers` supplies no-store/noindex
on Connect, including share management. All six canonical/legacy account, device
and share-management documents remain packaged. `_redirects` preserves legacy
hosted routes and their query strings. Anonymous capability recipients and their
media/manifest/preview/script/styles remain on the hosted Worker origin, not Pages.

The TypeScript/React source build is the production homepage. Packaging admits
only the public allowlist, and `compose-connect.ts` adds the independently built
Connect browser artifact. `prepare-pages.ts` adds provider metadata without
replacing homepage or asset bytes. `verify-composed.ts` verifies the source-built
homepage survives unchanged, validates Connect-owned bytes and both resource
graphs, and refuses private/unexpected paths or loopback production configuration.
Do not patch compiled bundles, maintain deployment overlays, or commit generated
site output. Original approved photos and media remain source assets.

The approved movie exceeds Pages' 25 MiB asset ceiling. A **Pages Function**, not a
separate Worker deployment or external origin, serves the existing movie URL from
small static chunks. It preserves the original bytes, audio, duration and range
seeking. Its `FixedLengthStream` supplies a real wire `Content-Length`: a manual
header on an ordinary stream is ignored by workerd, which can make the custom-domain
cache replace byte-range responses with full `200` responses and break seeking.
The workerd HTTP regression checks this boundary; Node response-header tests alone
do not. Only that exact movie path invokes the Function; other website assets
remain static. The account stays on its existing plan. Function requests consume
the existing Workers Free daily allowance; no paid service is activated. The project
is configured fail-closed. At quota exhaustion the movie may be unavailable; static
pages do not consume that Function allowance.

Public owns its locked esbuild, Miniflare and Wrangler dependencies.
`bun install --frozen-lockfile` in `public/site`, then `npm run package:public`
builds/packages the website without installing either Connect package or invoking
Cargo. `npm run build:pages-worker` compiles its Pages Function; the real workerd
movie test uses Public dependencies. Production composition still requires the
Connect browser artifact. The Pages publisher installs only Public dependencies.

## Independent Public development

Use Node 22.23+ and Bun 1.3.10. From `public/site`, `npm run verify` installs the
locked dependencies, builds, packages and runs Public's owner and publication
contract tests. Browser checks need installed Playwright Chromium or `CHROME_PATH`.
`npm run check:independence` copies only Public and its declared shared/content
inputs to a disposable source tree, denies Cargo/rustc/rustup/Python commands,
then verifies fresh installation, build, owner tests, packaging and the dev server.
It reuses no application/native build output and removes its temporary tree.
The production Pages movie test uses Public's prebuilt workerd distribution;
Public never compiles its native implementation. Connect composition still needs
its separately verified artifact; independence does not remove that contract.

Public's maintained code, tooling and Pages handler are TypeScript. The source
boundary test prevents Rust, Python, authored JavaScript and shell tooling from
returning. Generated browser/Worker JavaScript remains build output outside Git.
The unconsumed `video-demo` Rust renderer and its exclusive input assets were
retired after checking manifests, imports, scripts, workflows and package inputs;
current site demo/media and their TypeScript generation tools remain available.
Legacy generated-output exclusions stay bounded to `public/site/video-demo/output/`
so retained renders in older checkouts are not mistaken for maintained source.

## Shared value-stream CI

`public/delivery.json` registers Public in `Value-stream delivery` as `stream (public, web)`.
That lane installs locked Bun dependencies, builds/packages and tests Public once,
including the compiled Pages Function and its real workerd transfer checks.
It shares the common source/environment fingerprints, unchanged-artifact admission,
checksummed receipts and provenance artifacts. Public gets one Ubuntu web target;
it does not run a second Mac build for a platform-neutral website artifact.

`Website — checks & deploy` consumes the resulting immutable raw website through
`scripts/cd/stream-artifact.ts`. It requires successful Public, shared-contract and
runtime-audit jobs from the exact trusted producer attempt, candidate-compatible
source inputs and production configuration, and valid archive identities. It
accepts unchanged verified ancestor bytes and preserves receipt provenance outside
served content. A missing compatible artifact fails closed; no source-build fallback
exists. Common-workflow completion retries composition, as Connect completion does.
An unrelated failed component does not replace the required Public-specific proof.

The website workflow retains Connect admission, composition/resource/privacy and
browser checks, Pages movie transformation, exact-candidate sealing and publication.
It never recompiles either stream. New producer run receipts are required before
publication; a registered lane or passing local test is not a production receipt.

## Immutable Connect consumer contract

The consumer helper is `node --experimental-strip-types public/site/scripts/connect/artifact.ts .build/connect-browser`
from the repository root with an Actions-read `GH_TOKEN`. It accepts only retained
SHA-named browser artifacts from successful canonical-main `Connect producer`
runs, checks the Actions archive digest, then invokes Connect's published
`artifacts.ts validate` interface against the candidate. Reuse requires identical
complete build inputs and the currently approved production build configuration.
Absent, expired, corrupted or incompatible artifacts fail closed; this command
never installs or rebuilds Connect. The receipt records producer SHA, run, artifact
ID and digest outside served content.

Set `CONNECT_BROWSER_ARTIFACT` to the downloaded `dist/` directory for
composition and final verification. Existing local development defaults remain.
The production workflow consumes this artifact without installing, building or
retesting Connect product code. Public checks composition, route/resource integrity,
privacy metadata and the composed browser/analytics seam. Connect owns service and
credentialed browser product verification in its producer.

## Main-branch CD

Value-stream delivery contains a dedicated Public producer and deployment pair:
`stream (public, web)` → `deploy (public, web)`. Public is excluded from the
other-owner matrix. Deployment needs only the Public producer, which runs its own
delivery and shared architecture contracts before admitting or building bytes.
Unrelated owner jobs cannot hold Public deployment.

The producer uploads its sealed artifact and exposes its exact name and fingerprint.
Deployment downloads that same-run artifact and validates its revision, run/attempt,
source/configuration identity, inventory and checksums before extraction. It consumes
Connect's independently verified browser artifact; unavailable compatible Connect
output fails closed. Successful Connect completion retries Public delivery through
the common workflow. Neither stream is rebuilt by the composition step.

The deployment job runs existing composition, privacy, browser, theme and movie
checks, seals the exact candidate, and publishes under `WEBSITE_CD_ENABLED`, the
`website-production` environment and `website-production-eligible` lock. The
publisher retains source ordering and immutable/canonical readback verification.
The former website workflow no longer publishes Pages, preventing duplicate writers.

The independent Worker call uses Connect's reusable workflow and controls, triggered
by a successful producer completion and `HOSTED_CD_ENABLED`. It has no dependency
on Public verification. Its publisher owns `hosted-production` and the separate
`web-delivery-production-eligible` lock. Public's workflow neither installs the
service nor renders provider bindings, applies migrations or deploys Worker bytes.
No activation, provider, origin or consent settings change with this separation.

The artifact-only browser smoke intercepts every external transport and exercises
all six routes, legacy query-preserving aliases, anonymous API loading and browser
analytics consent. Its production seam uses only downloaded bytes. Public's common producer separately tests an enabled-analytics fixture in private temp
output without Connect; that fixture is never uploaded or published.
Existing Connect credentialed/browser suites remain owned by the producer.
`PUBLIC_BUILD_DIR` and `PUBLIC_SSR_DIR` select private build inputs;
`PUBLIC_PACKAGE_OUTPUT` selects a disjoint package output. Defaults retain the
normal paths. Packaging rejects output overlap with source inputs.

Configure `website-production` for `main` only, with secret
`WEBSITE_PAGES_API_TOKEN` (Pages edit for the existing account), then set repository
variable `WEBSITE_CD_ENABLED=true`. The migration reuses the authorized Pages token
stored in 1Password Hermes, `Apparent GitLab Cloudflare Pages Deploy 2026-08-21`;
its provider expiry is **2026-11-19T10:27:38Z**. Renew/replace that scoped token before
expiry and update the GitHub secret; never print it. The Pages publisher does not need
DNS, OAuth, R2, D1 or SSH credentials. Dispatch `web-delivery.yml` on main to retry.
A successful merge without a successful website job is not a production delivery.

## Historical hostname cutover

This records the accepted migration, not instructions to repeat it. The origin
retirement below supersedes any assumption that a Clanker baseline still exists.

Before writes, snapshot exact zone records, Workers routes/custom domains, Pages
projects/custom domains, Access applications and the active Clanker release.
Verify the Pages deployment URL first, including full movie hash, byte ranges,
Connect routes, redirects, public assets and private-path denial.

Add `app.tryapparent.com` as a custom domain of `apparent-capture`, then update only
its existing DNS record from proxied `A 148.251.88.146` to proxied
`CNAME apparent-capture.pages.dev`, TTL auto. Preserve its record ID, comment and
tags. Verify Pages domain/TLS status active, exact DNS readback, the Pages publisher
header, and ordinary plus cache-bypassed HTML/media bytes at the public hostname.

`connect.tryapparent.com` already aliases app in DNS. Preserve its existing
redirect-only behavior with the two narrowly host-scoped zone Redirect Rules in
`public/site/scripts/cd/connect_redirects.json`, device aliases before the catch-all. Snapshot
the zone dynamic-redirect entrypoint first; append only these rules and preserve
all existing rules byte-for-byte. Its DNS remains unchanged. Pages `_redirects`
rejects host-specific source URLs, so do not substitute invalid absolute-source
entries or turn every static request into a Function invocation. These redirects
retain all former device aliases and code queries; no Pages connect custom-domain
association or document-product rule change is required.
Do not change `hosted.tryapparent.com`, its Worker routes, API/auth services,
Google callback, cookies, OAuth configuration or credentials. Do not change
`tryapparent.com` or `www.tryapparent.com` or their Pages project.

## Origin retirement and rollback

The following acceptance and retirement outcomes are retained operator evidence
from 2026-11-01 (Europe/Berlin), not a fresh infrastructure check by the source-only
cleanup. Later source revisions do not establish a new production acceptance.

The canonical Pages cutover was accepted on 2026-11-01, from main
`6f043ff4ab4f4bdfa7e180c97ef7c37a397ff2ef`, Actions run `41252306148`, deployment
`476a95dd.apparent-capture.pages.dev`. Public-file hashes, full/range movie bytes,
first-load chapter seeking, mobile controls, Connect redirects and anonymous
credentialed session/CORS were checked at the canonical hostname. Real Google
sign-in and device approval still require human presence; these checks do not
claim an authenticated end-to-end session.

After acceptance, the obsolete `app.tryapparent.com` Caddy serving block, its
`apparent-site` symlink, historical capture-site releases and obsolete Connect
staging copy were removed from Clanker. There is no Clanker fallback. Do not
restore the old A record, recreate that symlink or run old overlay activators.
The shared `presentations-caddy-1` container, volumes, other Caddy routes and the
separate document-product deployment services remain: they are not capture-site
resources. The cooperative lock file remains to avoid splitting lock ownership.

For a bad subsequent Pages release:

1. Disable `WEBSITE_CD_ENABLED` and ensure no website deployment is in flight.
2. Read the current production deployment and recorded previously accepted
   deployment. Do not overwrite a newer operator's repair.
3. Use Pages' production-deployment rollback within `apparent-capture`. Keep its
   custom domain, DNS, Connect redirect rules and unrelated projects unchanged.
4. Verify the selected deployment ID, exact public bytes, private-path denial,
   video ranges/seeking and Connect behavior at `app.tryapparent.com` before
   re-enabling the gate.

The private cutover and retirement receipts remain outside served content at
`~/.local/share/apparent-deployments/pages-cutover/`. They are historical evidence,
not permission or a procedure to reactivate the retired origin. Never publish
their inventories. Do not roll back hosted databases or the auth Worker with this
site.

## Verification

Run existing public-site tests on the source build and final package, Pages
package/video and Public CD tests, and the artifact-only composed browser and
analytics smoke. Connect's producer owns its credentialed synthetic browser suite. On both the immutable deployment and custom hostname run the recorded-video
browser verifier and inspect desktop/mobile screenshots, frog jumps/waves, download
destinations and all lazy feature frames. Check 206 Content-Range and full movie
SHA-256. Check noindex/no-store on Connect, preserved legacy query strings, no
private-route fallback, local fonts and all referenced assets. Anonymous live
session/CORS and sign-in redirects can be checked without accounts. Real Google
sign-in, authenticated cookies, device approval and logout require human presence;
never invent accounts or approve a device merely to finish deployment acceptance.

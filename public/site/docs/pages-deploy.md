# Capture website delivery on Cloudflare Pages

The canonical repository is `https://github.com/tryapparent/apparent`, branch
`main`. `.github/workflows/web-delivery.yml` is the sole production publisher for
`app.tryapparent.com`. It publishes the isolated **apparent-capture** Pages project
in account `a54dcebead3b8ad688f891da0ab54d1f`. It does not publish the document-product
project `tryapparent-website`, the hosted API Worker, or `public/site/wrangler.jsonc`.

## Public artifact and accepted design

Build the two owners independently with Bun 1.3.10 and Node 22.23.0, with
locked dependencies installed in each package. From the repository root:

```sh
(cd connect/web && bun install --frozen-lockfile && VITE_HOSTED_API_ORIGIN=https://hosted.tryapparent.com npm run build)
cd public/site
APPARENT_PUBLIC_SITE=1 npm run build
node scripts/site-build/package-apparent.mjs
node scripts/connect/compose-connect.mjs
node scripts/pages/prepare-pages.ts
node scripts/site-build/verify-composed.ts
node scripts/pages/prepare-pages-video.mjs
./node_modules/.bin/esbuild lib/pages-worker.ts --bundle --platform=browser --format=esm --outfile=dist-apparent/_worker.js
PAGES_WORKER=dist-apparent/_worker.js node --test scripts/pages/test-pages-video-runtime.mjs
python3 scripts/cd/publish_pages.py seal <full-commit-sha>
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
only the public allowlist, and `compose-connect.mjs` adds the independently built
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

## Main-branch CD

The workflow starts directly on main pushes or manual main dispatches, independently
of native application verification. Candidate admission validates canonical-main
event and checkout identity. Both publishers require the same run's complete web
verification; failed or skipped web checks cannot admit either publisher. Pages
consumes the revision-named sealed artifact. The hosted publisher currently
rebuilds its Worker from that verified SHA before publication. Public and Connect currently share the composed
Pages artifact and web verification lane; this does not claim separate deployment
of those two owners yet.

PR verification has no production credentials. The production Pages artifact is
frozen before the separate synthetic Connect build. The website job downloads only that run's revision-named artifact, checks current
main, verifies its complete checksum inventory, and deploys without bundling. The
credential-free job compiles the Pages Function into `_worker.js` and tests that
compiled worker in workerd before sealing `.build/website/`. This artifact contains
`dist-apparent/`, `wrangler.jsonc`, and a nonserved `manifest.json` binding every
file to the admitted commit. Artifacts are retained for 90 days; provider deployment
IDs and readback receipts identify rollback candidates independently of that TTL.
The production job copies no Function source or repository library into staging.
It checks HTML and emitted JS/CSS at both the immutable deployment URL and canonical
hostname, plus movie identity and Connect privacy headers. The receipt records
revision, deployment ID and manifest digest. Publication is serialized without
canceling an in-flight job. The `hosted` job and its activation gate are unchanged.

The synthetic integration build uses `connect/web/dist-test` and writes only
`public/site/dist/synthetic-composition`. `COMPOSE_SYNTHETIC_ORIGIN` must name its labelled
loopback API, and `COMPOSE_PUBLIC_INPUT` points at the saved Public package before
composition. Production composition has no synthetic-origin exception. Never upload
either test directory. Package fixtures cover the independent owner; the retained
site browser suite covers the composed routes and #191 sharing interactions.

Configure `website-production` for `main` only, with secret
`WEBSITE_PAGES_API_TOKEN` (Pages edit for the existing account), then set repository
variable `WEBSITE_CD_ENABLED=true`. The migration reuses the authorized Pages token
stored in 1Password Hermes, `Apparent GitLab Cloudflare Pages Deploy 2026-08-21`;
its provider expiry is **2026-11-19T10:27:38Z**. Renew/replace that scoped token before
expiry and update the GitHub secret; never print it. The workflow does not need
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

Run existing public-site tests on the source build and final package, then the Pages
package/video tests, CD Python tests and Connect's credentialed synthetic browser
suite. On both the immutable deployment and custom hostname run the recorded-video
browser verifier and inspect desktop/mobile screenshots, frog jumps/waves, download
destinations and all lazy feature frames. Check 206 Content-Range and full movie
SHA-256. Check noindex/no-store on Connect, preserved legacy query strings, no
private-route fallback, local fonts and all referenced assets. Anonymous live
session/CORS and sign-in redirects can be checked without accounts. Real Google
sign-in, authenticated cookies, device approval and logout require human presence;
never invent accounts or approve a device merely to finish deployment acceptance.

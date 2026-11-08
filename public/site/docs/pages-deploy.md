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
task_public_input=$(mktemp -d)
cp -a dist-apparent/. "$task_public_input/"
export COMPOSE_PUBLIC_INPUT="$task_public_input"
node scripts/connect/compose-connect.mjs
node scripts/pages/prepare-pages.mjs
node scripts/site-build/verify-composed.mjs
npm run build:pages-worker
PAGES_WORKER=dist-apparent/_worker.js node --test scripts/pages/test-pages-video-runtime.ts
node scripts/pages/prepare-pages-video.mjs
SITE_ARTIFACT=dist-apparent node --test scripts/pages/test-public-homepage.ts
```

Upload only `dist-apparent`. Never upload `dist`, this repository, evidence, private prototypes, pitches, internal feature maps or
operator metadata. `404.html` disables Pages' implicit SPA fallback. The preparation
step checks every input path against the former Caddy public boundary, including
only the three approved hosted illustrations. `_headers` supplies no-store/noindex
on Connect, including share management. All six canonical/legacy account, device
and share-management documents remain packaged. `_redirects` preserves legacy
hosted routes and their query strings. Anonymous capability recipients and their
media/manifest/preview/script/styles remain on the hosted Worker origin, not Pages.

The homepage is now built directly from Public's TypeScript/React sources. The
recovered source includes the photo layout, mobile examples, responsive movie
camera, timeline and frog QR interaction. The old checked-in HTML/JS snapshot and
compiled-bundle overlay tools are retired; editing the source changes the packaged
homepage. Simon approved publishing this design on 2026-11-08 after comparison
confirmed it matches the live site. The inherited branding-only hold is cleared;
normal exact-commit verification and publication gates remain required.

To minimize recovery scope, delivery retains main's existing composed Pages
arrangement and manual release gates. Public imports no Connect implementation.
`compose-connect.mjs` consumes the independent browser artifact and checks its
six documents, resource graph, production API origin and five canonical assets.
It refuses private paths, symlinks, missing resources and differing collisions
before changing the destination. `verify-composed.mjs` compares every saved Public
and Connect byte after metadata preparation; there is no homepage overlay exception.
Keep `COMPOSE_PUBLIC_INPUT` pointed at the saved Public package until verification
finishes. The synthetic fixture uses the same boundary with its labelled origin.
No Cloudflare routing cutover, migration-wide delivery framework or service
activation is part of this source recovery.

Standalone source and browser checks: `cd public/site && npm run verify`.
The locked website toolchain owns its movie compiler, workerd regression and Pages
publisher; it does not install Connect's service to obtain those tools. The manual
combined workflow still verifies Connect separately before publication.

The approved movie exceeds Pages' 25 MiB asset ceiling. A compiled **Pages Function**, not a
separate Worker deployment or external origin, serves the existing movie URL from
small static chunks. It preserves the original bytes, audio, duration and range
seeking. Its `FixedLengthStream` supplies a real wire `Content-Length`: a manual
header on an ordinary stream is ignored by workerd, which can make the custom-domain
cache replace byte-range responses with full `200` responses and break seeking.
The workerd HTTP regression checks this boundary; Node response-header tests alone
do not. Verification emits `dist-apparent/_worker.js`, exercises that exact compiled worker
in workerd and in browser playback, and publishes it with `--no-bundle`. Only that
exact movie path invokes the Function; other website assets
remain static. The account stays on its existing plan. Function requests consume
the existing Workers Free daily allowance; no paid service is activated. The project
is configured fail-closed. At quota exhaustion the movie may be unavailable; static
pages do not consume that Function allowance.

## Main-branch CD

PR verification has no production credentials. The production Pages artifact is
frozen before the separate synthetic Connect build. The website job downloads only
that run's revision-named artifact, checks current main, deploys with the exact
commit hash and reads back the production deployment plus root/Connect bytes.
Publication is serialized without canceling an in-flight job. The `hosted` job and
its activation gate are unchanged and must remain disabled for this website task.

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

Run Public source/browser tests before composition, then preservation checks,
compiled-worker playback, Pages package/video tests, CD Python tests and Connect's
credentialed synthetic browser suite. On both the immutable deployment and custom hostname run the recorded-video
browser verifier and inspect desktop/mobile screenshots, frog jumps/waves, download
destinations and all lazy feature frames. Check 206 Content-Range and full movie
SHA-256. Check noindex/no-store on Connect, preserved legacy query strings, no
private-route fallback, local fonts and all referenced assets. Anonymous live
session/CORS and sign-in redirects can be checked without accounts. Real Google
sign-in, authenticated cookies, device approval and logout require human presence;
never invent accounts or approve a device merely to finish deployment acceptance.

## Website analytics

The recovered website helper preserves its own affirmative visitor choice,
withdrawal and GPC/DNT handling. Its build uses `PUBLIC_POSTHOG_ENABLED`,
`PUBLIC_POSTHOG_TOKEN`, `PUBLIC_POSTHOG_HOST` and `PUBLIC_POSTHOG_ENVIRONMENT`.
Capability defaults enabled, but absent/invalid public ingestion configuration
fails closed. This recovery does not change repository/provider variables or
Connect account analytics. Verification uses intercepted synthetic events; no
fixture event is sent to PostHog. Production configuration and publication remain
subject to the existing gates.

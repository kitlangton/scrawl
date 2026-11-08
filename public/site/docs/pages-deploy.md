# Public delivery on Cloudflare Pages

The canonical repository is `https://github.com/tryapparent/apparent`, branch
`main`. `.github/workflows/stream-public.yml` is the sole production publisher for
`app.tryapparent.com`. It publishes the existing **apparent-capture** Pages project
in account `a54dcebead3b8ad688f891da0ab54d1f`. It does not deploy Connect, the hosted
API Worker, Clanker, or the separate document-product website.

## Build and checks

Public uses Bun 1.3.10 and Node 22.23.0. From `public/site`:

```sh
bun install --frozen-lockfile
./node_modules/.bin/playwright-core install --with-deps chromium firefox
npm run verify
node scripts/cd/publish-pages.ts seal "$(git rev-parse HEAD)"
```

`verify` builds the source homepage, packages its allowlisted assets, tests the
resource graph, browser behavior, analytics consent and shared appearance, compiles
and exercises the movie worker in workerd, then prepares Public-only Pages metadata
and movie chunks. Public's theme check includes Public and shared inputs. It does
not install or build Connect, Rust or Desktop. Shared-token changes still require
the affected consumers' own acceptance; Public's checks do not certify those products.

The homepage includes the recovered photo layout, responsive movie, timeline and
frog QR interaction. Simon approved publication on 2026-11-08 after comparison
confirmed it matched the live site. No snapshot or bundle overlay replaces the
source build. Analytics configuration and visitor consent remain unchanged.

The sealed `.build/website` artifact contains only `dist-apparent`, the isolated
Pages configuration and a revision/checksum manifest. The publisher revalidates its
complete inventory, approved movie chunks, metadata and revision before staging it.
Private files, evidence, source maps, application documents and Connect/hosted
outputs are rejected. `404.html` disables Pages' implicit SPA fallback.

The movie exceeds Pages' 25 MiB asset ceiling. Its compiled Pages Function serves
the existing URL from verified static chunks, preserving full bytes and byte
ranges. `FixedLengthStream` supplies a real wire `Content-Length`; the workerd
regression exercises this rather than relying on Node response headers. The
verified `_worker.js` is published with `--no-bundle`. Only that exact movie path
invokes the Function. The existing Free plan and fail-closed quota behavior stay
unchanged; no separate Worker or paid service is activated.

## Checks and manual publication

Public checks run on relevant PRs and main pushes. **Only a manual dispatch on
canonical `main` can publish.** Dispatch `stream-public.yml`; it runs Public's
checks and downloads only that run's SHA-named sealed artifact into the deploy
job. It uses the existing `website-production` environment, `WEBSITE_CD_ENABLED=true`
gate and scoped `WEBSITE_PAGES_API_TOKEN`. No new secrets or provider configuration
are required. The token's recorded expiry is **2026-11-19T10:27:38Z**; renew it
through the existing secret workflow before expiry without printing it.

Public does not require a full Linux/shared verification receipt or a Connect
run. Publication is serialized separately from checks and cannot be cancelled by
a new push. Immediately before upload, the publisher requires current main and
refuses to overwrite an equal or newer deployed revision. A superseded candidate
skips publication without a new receipt; dispatch current main again when needed.
Readback verifies the revision and Public HTML, JS/CSS and full movie bytes at both
the immutable deployment URL and `https://app.tryapparent.com`. Merge success alone
is not production delivery.

## Connect boundary

Connect's runtime, API/auth, database, OAuth, DNS, Worker routing, activation gate
and Linux/shared release admission are unchanged. Its account, device and shares
routes are already served by its independent Worker at `app.tryapparent.com`;
legacy `/hosted` redirects and `connect.tryapparent.com` aliases retain their code
queries there. Public uploads no Connect documents, redirects or application
assets. No routing cutover is part of this split.

`web-delivery.yml` retains Connect's existing service and cross-origin browser
checks and its separately gated hosted deploy job. Those integration fixtures
still compose Public and Connect locally; they cannot publish Pages. Public's own
checks and publication do not depend on that workflow.

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

Run `npm run verify` and seal the final artifact. After deployment check both the
immutable URL and canonical hostname: Public graph bytes, full movie SHA-256,
206 Content-Range, private-path denial and recorded-video playback. Compare
Connect account/device/shares bytes and privacy headers, legacy redirects and
code queries against their pre-deployment state. The immutable Public deployment
has no Connect application; the canonical Worker routes must remain intact.
Anonymous live session/CORS and sign-in redirects can be checked without accounts.
Real Google sign-in, authenticated cookies, device approval and logout require
human presence; do not invent accounts or approve a device for a website deployment.

## Website analytics

The recovered website helper preserves its own affirmative visitor choice,
withdrawal and GPC/DNT handling. Its build uses `PUBLIC_POSTHOG_ENABLED`,
`PUBLIC_POSTHOG_TOKEN`, `PUBLIC_POSTHOG_HOST` and `PUBLIC_POSTHOG_ENVIRONMENT`.
Capability defaults enabled, but absent/invalid public ingestion configuration
fails closed. Public verification builds use the existing repository ingestion
secret `APPARENT_POSTHOG_TOKEN` with the EU host and production environment.
Pageviews include PostHog's standard URL fields built only from approved routes;
queries, fragments and private routes are excluded. This does not change
Connect account analytics. Verification uses intercepted synthetic events; no
fixture event is sent to PostHog. Production configuration and publication remain
subject to the existing gates.

# Public website delivery

Public builds, verifies and deploys its own website through
`.github/workflows/stream-public.yml`:
`stream (public, web)` → `deploy (public, web)`. Deployment needs only its producer.
The canonical repository is `tryapparent/apparent`, branch `main`.

## Ownership and cutover

Public owns the homepage, marketing assets, feature demos and approved movie on
`app.tryapparent.com`. Its existing `apparent-capture` Cloudflare Pages project
(account `a54dcebead3b8ad688f891da0ab54d1f`) remains the Public target.

Connect independently owns the canonical /connect pages, /connect/v1/ API,
/connect/auth/ sign-in, /connect/s/ recording links and their resources.
The existing apparent-hosted-beta Worker serves app.tryapparent.com/connect*
and /hosted*; legacy aliases preserve queries and the old API transport.
Public neither deploys Connect bytes nor changes its provider resources.

The Connect owner verified the authorized live cutover from source
4cad1dd5d626bf0e55bfad48ffc25c2aec5548d5, run 42171680833, artifact
13481396964, Worker version e12adf28-b174-4191-8acf-a7c132e45f14.
Real Google callback/session/logout checks, canonical pages/assets, privacy and
provider resource preservation cleared the former Public cutover hold.
Installed native-client publication is a separate adoption gate.

After Public-only publication, retain the independent app Worker routes.
Restore a known-good gateway version for Connect rollback, or restore combined
Pages before removing those routes. Removing them against a Public-only Pages
snapshot would make Connect unavailable. Public publication/readback still needs
its own exact-main evidence.

## Build and artifact admission

Use Node 22.23.4 and Bun 1.3.10. `public/delivery.json` declares Public/shared/content
inputs and Public analytics build configuration. It has no Connect API-origin or
browser-artifact dependency. `npm run verify` installs locked dependencies, builds,
packages and tests Public, including its compiled Pages movie handler and a private
enabled-analytics fixture. Synthetic fixture bytes are never uploaded.

`npm run check:independence` verifies a fresh source copy with Cargo/Rust/Python
denied. The producer uses `verify:installed`, installs its locked Chromium and
Firefox, checks delivery/shared architecture contracts and seals its component.
Browser verification covers raw output and an owned temporary chunked copy;
the latter serves movie requests through the compiled Pages worker in workerd.
Unchanged artifacts use the shared receipt/admission protocol.

Deployment downloads the exact same-run component and checks its producer output
fingerprint, source/configuration, revision, run/attempt and archive inventory/hash.
Only Public bytes are extracted. It installs only Public's locked tooling, checks
its resource graph and canonical assets, verifies preservation during metadata
preparation, exercises browser/theme/movie behavior, then seals the candidate.
The Public allowlist rejects Connect/hosted output even with a matching manifest.
There is no Connect fetch, source build, composition, retry trigger or fallback.

For local verification after obtaining a raw Public artifact:

```sh
cp -a /path/to/raw-public public/site/dist-apparent
cd public/site
node scripts/pages/prepare-pages.ts
PUBLIC_ARTIFACT_INPUT=/path/to/raw-public node scripts/artifact/verify.ts
node scripts/pages/prepare-pages-video.ts
SITE_ARTIFACT=dist-apparent node --test scripts/pages/test-public-homepage.ts
node scripts/cd/publish-pages.ts seal <full-main-commit-sha>
```

Sealing is local and does not publish. Actual publication requires the canonical
value-stream workflow's `deploy` job on main, `WEBSITE_CD_ENABLED`, its existing
`website-production` environment/Pages token and `website-production-eligible` lock.
Only this job invokes the publisher; a workflow regression rejects other entrypoints.
Retry the existing Public owner run on main under its normal gates. Publication
retains source ordering, exact-candidate checksums and live readback of Public HTML,
JS/CSS and movie bytes at both the deployment URL and canonical alias. The receipt
records the exact revision, deployment ID/URL and manifest digest.

## Static boundary and movie

Upload only the sealed Public package. Never upload the repository, raw `dist`,
private maps/evidence, transcripts, captures, credentials or synthetic fixtures.
A real `404.html` disables SPA fallback for unknown paths. Public metadata contains
no Connect redirects or privacy rules; those belong to Connect's independent
routing target. Existing Connect shortcut routing is preserved by that owner.

The approved movie exceeds Pages' 25 MiB asset ceiling. Public's compiled Pages
handler serves the unchanged movie URL from four checksum-bound chunks, preserving
full/range responses, wire length, audio and seeking. Only that movie path invokes
the handler. Keep the workerd transfer regression; Node header checks alone do not
prove the boundary. The account's existing free allowance and fail-closed quota
behavior remain unchanged; no new paid resource is activated.

## Recovery and retained boundaries

Disable `WEBSITE_CD_ENABLED` and ensure no Public publication is in flight before
an authorized Pages rollback. Keep source ordering and readback checks intact.
The retired Clanker capture-site publisher must not be restored; unrelated shared
proxy sites, volumes and services remain outside Public scope. See
`clanker-deploy.md` for the retained retirement constraints. Preserve provider
records/identity and use existing approved access for credential renewal; never
print or commit the Pages token. A merged PR or skipped deployment is not proof
of publication. Missing routing-cutover or actual deployment evidence remains an
explicit acceptance gap.

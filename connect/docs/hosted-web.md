# Connect web

Canonical routes are `https://app.tryapparent.com/connect`, `/connect/device?code=USERCODE`, and `/connect/shares`, including slash/no-slash variants. The legacy Public composition retains its six `/hosted` rendering aliases until cutover. The dedicated Worker instead redirects only exact GET/HEAD requests for `/hosted[/]`, `/hosted/device[/]`, `/hosted/shares[/]`, and the three existing `/hosted/{source-pond-v4,agent-pond-v4,agent-reading-v4}.png` links. It preserves query bytes, including device codes, and redirects to the fixed deployment `SITE_ORIGIN` canonical `/connect` equivalents. These are temporary **302** redirects with `no-store`, `no-referrer`, `noindex`, and no cookies, so rollback does not leave a sticky cached redirect. Other methods receive 405; unknown, encoded, private and near-prefix paths receive 404. No Connect bytes are copied into Public by this compatibility bridge. The aliases remain absent from the homepage and sitemap. This source adoption is not deployment or real sign-in proof.

The setup dialog uses `apparent connect login` and `apparent connect status`.
It tells users to update Apparent for `connect`; older apps can use `apparent cloud`,
retained as a compatibility alias. Publishing the website does not update the app.

## Backend contract and boundaries

The frontend is aligned to `connect/docs/hosted-api.md`; `connect/web/src/api.ts` isolates the wire contract:

- `GET /v1/session`: anonymous `{authenticated:false,...}` or authenticated `{authenticated:true,csrfToken,account:{email,beta:'active'|'waitlist',waitlist:{interestedAtFiveDollars:boolean},...},slots:{used,limit:20,remaining}}`. Invalid or missing counts do not render invented availability. An account marked `waitlist` is already on the authenticated waitlist; the form updates its optional interest.
- `GET /auth/google/start?returnTo=...`: only fixed `/connect` or `/connect/device?code=...` destinations are generated, including from legacy pages. Arbitrary incoming `returnTo` is ignored. Callback processing belongs entirely to the Worker; it also normalizes pre-transition stored `/hosted` paths safely.
- `POST /auth/logout`, `POST /v1/waitlist {interestedAtFiveDollars:boolean}`, and `POST /v1/device/approve {userCode}`: credentials included, `X-CSRF-Token` from session, JSON body. The browser supplies Origin. After mutations the UI reads the exact session/device state back before displaying success.
- `GET /v1/device?userCode=...`: authenticated preview `{scope:'read'|'upload',status:'pending'|'approved'|'rejected'|'expired'}`. **Requested API improvement: include ISO `expiresAt`** so the screen can show actual expiry and reject a newly expired code locally. Until available, the UI states the documented ten-minute lifetime, handles server `expired` status, and leaves enforcement to the API. Optional `expiresAt` is supported and validated. Preview never approves anything; only an explicit confirmation click does.
- Upload permission includes add/read/delete of account captures, per backend contract. Agent `read` is a separate read-only credential. Unknown scopes fail closed. No browser endpoint should return device secrets or bearer tokens.

The canonical deployment profile uses `VITE_HOSTED_API_ORIGIN=https://app.tryapparent.com/connect`, `API_ORIGIN=https://app.tryapparent.com/connect` and `SITE_ORIGIN=https://app.tryapparent.com`. UI, API, authentication callbacks, capability links and viewer assets share this origin and mount. Browser configuration accepts only a deployment-controlled HTTPS origin or exact `/connect` base (loopback HTTP for local tests), without credentials, query or fragment. This is public configuration, never an OAuth secret. CORS and CSRF still compare exact origins, with HttpOnly/Secure/SameSite cookie protections. The currently deployed legacy profile uses `https://hosted.tryapparent.com` for the API and host-only cookies; retain it only for compatibility until the canonical gateway and provider callback are accepted. Public Pages composition is a legacy deployment path, not the canonical target.

Better Auth receives the API URL's origin as `baseURL` and the full mounted `/connect/auth` path as `basePath`. A path-bearing `baseURL` makes Better Auth ignore `basePath`, breaking callback dispatch even when sign-in initiation works. The artifact verifier builds the service and runs its focused OAuth/mount tests, including the signed provider callback and authenticated session readback, before delivery can seal the artifact.

Session/CSRF data stays in memory, never localStorage or DOM attributes. The human code is only passed to fixed approval endpoints and the fixed sign-in return route, never treated as a capture permission. There is no token input, private capture browser, web recorder, billing, or browser share-creation control. Capture authorization must be enforced by the API; knowing an ID is not access.

### Better Auth migration candidate (#187)

Connect's service owns Better Auth and the compatibility endpoints above. The
browser keeps credentialed requests and explicit scope confirmation; it does not
receive or renew native access/refresh tokens. Updated native clients renew each
device's approved scope independently. Browser sign-in or logout is not native
device approval or revocation, and account authentication never enrolls a trusted
decryption key. Revoked or expired renewal requires a new CLI approval; temporary
network failure does not. Existing credentials need an updated client and one new
login. The limits disclosure states this distinction instead of promising a
monthly reapproval cycle. See the API/client contracts for lifetimes and migration.
Synthetic browser fixtures verify the disclosure and unchanged consent/readback
controls, not actual Better Auth, Google OAuth or deployed cookie behavior.

## Single-recording sharing candidate (#171)

The native off-by-default sharing experiment owns sender visibility. The backend separately gates capability sharing; browser assets do not enable it, start automatic native uploads, or provision resources. Connect links to **Manage shared links** at `/connect/shares` (legacy `/hosted/shares`), which reads `GET /v1/shares` with the existing browser session and follows `nextCursor` until the complete list is loaded. Each response is `{shares:[{id,captureId,url?,status,createdAt,expiresAt,title}],nextCursor:string|null}`. Only live, unexpired rows expose a validated HTTPS URL. Titles are captured evidence and render as text, never HTML. Empty, unavailable, anonymous and denied states do not invent links. A single-capture link is a read capability for that capture only, not an account-wide device grant.

**Stop sharing** sends `DELETE /v1/shares/:id` with included cookies and the in-memory `X-CSRF-Token`; the browser supplies its actual Origin, and the backend must enforce the exact trusted Origin. The button blocks duplicate submissions. The UI reads the complete paginated list back and checks that exact ID is gone or terminal before displaying the resulting state. A successful DELETE without confirmed revocation shows an error, not success. This revokes the single-capture capability, not a device grant, and never deletes the private capture. Device-grant management and revocation UI remain deferred. The page explains that anyone with a link can view/download and that revocation cannot recall downloads or cached previews. Anonymous visitors sign in through the fixed canonical `/connect` return destination, then reopen management; no arbitrary return target is introduced. The Account link returns to `/connect/` even from the legacy management page.

`connect/service/src/share-view.ts` exports `renderShare(model) -> HTML`, without fetching. The model is `{url,status,expiresAt,uploadedBytes,totalBytes,title,summary,transcript,files:[{path,size,sha256,url}],thumbnailUrl}`. Ready evidence requires both `status:'ready'` and equal valid byte counters. Pending, paused, failed, expired, revoked and deleted pages do not render captured text, media URLs or previews. The backend remains the authority for completion, retention and access at every endpoint.

The recipient frontend is React + TypeScript with the canonical shadcn `Button` and Tailwind v4 tokens from `shared/ui`. It lives in `connect/service/src/recipient`, separate from the authenticated account app. `npm ci && npm run build` in `connect/service` builds its browser IIFE, edge-compatible SSR renderer and CSS into ignored `.generated/` before compiling the Worker. No Node server runs in production. Refresh the `file:` shared UI snapshot with `npm ci` when canonical UI sources change.

Ready HTML places browser-native video beside timestamped transcript passages; phone layouts place the transcript below. Clicking a passage seeks without changing play/pause. Before metadata loads it asks the viewer to retry. There are no chapter controls, generated summary/title, frame gallery, transcript search/editing, auto-scroll or custom playback controls. Missing transcript/video states retain original evidence access. Use only existing title metadata or the generic title.

**Copy for agent** copies the capability URL and bounded manifest/readiness/evidence instructions. Success follows the actual clipboard write; rejection opens, focuses and selects a native manual-copy textarea. Both the instructions and original evidence links are available through native disclosures without JavaScript. Frame evidence links show capture timestamps from matching apparent.json keyframes when valid metadata is available; missing or invalid timestamps leave filename-only links. Labels are server-rendered and need no JavaScript. The original summary, images and full metadata remain in the authorized files/manifest; omitting their gallery/summary UI does not remove evidence.

`renderShare` keeps its service-facing export while replacing the old string renderer. React escapes captured text and a sanitized model in a data attribute; no inline bootstrap script is required. Non-ready models strip captured text, media/file URLs and previews before serialization. Ready asset URLs must match this exact capture's file path. Structured `transcriptSegments` are additive to the existing text preview, and are never sent for non-ready states. No account data or credentials enter the viewer bundle. Copy for agent asks the receiving agent to carry out the sender's transcript instructions rather than stopping at a summary. The accompanying user request determines the task; incidental screen text remains evidence, and existing permission and approval requirements still apply.

The same Worker serves four public build assets: `/share-view.js` (`text/javascript`), `/share-view.css` (`text/css`), `/share-wordmark.svg` (`image/svg+xml`) and `/share-font.ttf` (`font/ttf`). Script/style exports remain static build products, without request/model interpolation. CSP keeps `default-src 'none'`, `script-src 'self'`, `style-src 'self'`, `font-src 'self'`, `img-src 'self'`, `media-src 'self'`, `connect-src 'self'`, `base-uri 'none'` and `frame-ancestors 'none'`. No inline exceptions or third-party resources. Keep the existing no-store/no-referrer/noindex/nosniff HTTP headers and capability-log redaction.

Routing/rollback: the existing capability Worker remains the only recipient entry. This replacement adds no site/account routing or activation flag. Roll back the scoped Worker/frontend commit together. The legacy string renderer is removed with its consumers after the replacement's built SSR, browser, CSP and authorization checks pass; there is no parallel viewer. Existing sender/backend experiment and deployment gates remain unchanged.

Encryption boundary: this viewer consumes the existing service-readable single-capture contract. Private encrypted captures remain refused by the service until the separate, explicitly authorized shared-copy integration is implemented. This frontend slice does not turn encrypted bytes into public evidence, introduce private key escrow, create a share or grant access on page load. Shared copies use HTTPS and R2's Cloudflare-managed encryption at rest, not end-to-end encryption. See #171/#189 for the accepted integration scope.

Pending HTML explains bounded manifest checks even without JavaScript. Its external script requests only the same-origin manifest, without account cookies or referrers, every five seconds, with a request timeout, at most 24 attempts and a two-minute wall-clock deadline. Ready or terminal transitions reload server-rendered truth. Failures remain pending, and exhausted checks ask the recipient to reload later or contact the sender. No countdown or upload ETA is fabricated.

## Browser entry ownership

`connect/web` independently builds its browser entry and loading-only SSR. Its
five behavior/style donors were compared byte-for-byte with main `d59796c3`
before removal from `public/site/src/hosted`. StrictMode, hydration, local font, resets,
V4 art and `rgb(2, 1, 2)` text remain unchanged. Public imports no sibling Connect
source; `public/site/connect.html`, Connect-specific Public Vite rewrites/input and Public
`renderHosted`/prerender generation are retired together with those donors.

Connect's own dev/preview middleware and Public composition retain canonical and
legacy account, device and shares paths. The dedicated Worker package takes
only the three canonical pages and their emitted JS/CSS resource closure; it
does not include `/hosted` aliases or become the Public Pages publisher. Its
route-free template is a build artifact only. The existing legacy producer and
deployment path remain unchanged until compatibility is adopted separately.

## Verification

For the canonical combined artifact, install the hosting package with locked npm dependencies and run its verifier with Bun 1.3.10 available:

```sh
npm --prefix connect/tooling/hosting ci --include=dev
VITE_HOSTED_API_ORIGIN=https://app.tryapparent.com/connect API_ORIGIN=https://app.tryapparent.com/connect SITE_ORIGIN=https://app.tryapparent.com npm --prefix connect/tooling/hosting run verify:artifact
```

The verifier installs locked web/service dependencies, builds the browser and existing service implementation, packages the gateway, and tests the compiled artifact with local ASSETS and synthetic D1/R2/auth bindings. Wrangler runtime tests load the deployable config and compiled bytes with bundling disabled. Tests cover UI hydration, anonymous session, account denial, OAuth callback URI/cookies, viewer resources and static privacy boundaries. They do not establish real provider sign-in or live deployment. Evidence stays in private temporary directories. Runtime tests use an unchanged copy of the deployment package outside `.output`; Wrangler state/cache directories never enter the sealed artifact. Tests stop the runtime process group before removing that copy, and a final inventory guard permits deployment files only. Run one build/verifier at a time per checkout.

The legacy Public composition profile still uses the origin-only API configuration until cutover. Its publisher and compatibility graph checks remain documented in `public/site/docs/pages-deploy.md`; do not use those legacy commands to prepare the canonical gateway artifact or publish synthetic fixtures. Browser, service and recipient tests remain available in their owning packages.

## Release prerequisites

The existing capture frontend remains on Cloudflare Pages until its compatibility cutover; see `public/site/docs/pages-deploy.md` for that publisher's rollback. Preserve all six canonical/legacy Connect account/device/share-management documents, the three exact `/hosted/*-v4.png` art paths, hashed `/assets/` modules, original font/wordmark, accepted homepage and retained public assets without broadening the legacy public boundary. Legacy redirects preserve query strings, including device codes; source rendering aliases remain packaged. This hosting slice instead builds the same-origin Worker artifact described below, where anonymous capability recipients and account/API routes reach the existing service handler under `/connect`. No routes, bindings or deployment are activated. Synthetic local checks are not live OAuth or live-cookie proof, and packaged files do not prove that a host serves them. Keep these routes unlinked from the public homepage until beta deployment is verified. No database migration, provisioning or credential rotation is part of this work.

## React recipient verification

From `connect/service`, run `npm ci`, `npm run build`, then the affected backend tests (`node --test --test-concurrency=1 test/sharing/*.test.mjs test/access/encryption.test.mjs`). `npm test` retains the full service suite. Source renderer tests and Public's composed browser fixtures require the generated recipient renderer from the service build first.

For actual local Worker + D1/R2 browser checks, run `npm run test:recipient` with installed Chromium (`CHROME_PATH`) and FFmpeg. Set an existing private `RECIPIENT_EVIDENCE` directory or let the check create one under the OS temp directory. It generates synthetic four-second media, uploads it through the actual service, exercises pending→ready, native playback, transcript seek, actual clipboard and refusal fallback, no-JS evidence, responsive layout and revocation. No real account or cloud service is contacted. Screenshots and receipts remain outside git; these do not establish deployed hosting, ChatGPT-web retrieval or native Desktop acceptance.

## Same-origin Connect hosting source

`connect/tooling/hosting` builds one `apparent-connect-web` Worker. It serves
the canonical `/connect`, `/connect/device` and `/connect/shares` documents,
then dispatches `/connect/v1/*`, `/connect/auth/*`, `/connect/s/*` and recipient
viewer assets to the existing `connect/service/src/worker.ts` module. This
preserves the service's authorization, D1/R2 access, OAuth cookies, CSRF/CORS
and privacy headers. There is no generic proxy or SPA fallback: only exact
browser routes and packaged `/connect/assets/*` files
reach ASSETS, with credentials stripped first. The service's existing
scheduled handler remains the single cleanup implementation. The Wrangler
template has no routes, preview exposure, provider resource IDs or credentials;
operator configuration and activation remain separate.

The gateway also preserves the existing API/auth/viewer contract on the exact
`https://hosted.tryapparent.com` host: `/v1[/...]`, `/auth[/...]`, `/s/...`, and
the four recipient viewer assets dispatch unchanged to the service under its
legacy root-origin profile. The legacy profile is cached per runtime environment
and refreshed when its bindings or secrets change, preserving Better Auth's
environment-scoped initialization behavior. Other hosts and unknown legacy
paths do not reach the service. Scheduled cleanup delegates once to that same
service handler; deployment into the existing Worker must retain its current
bindings and sole cron. This is synthetic compatibility evidence only; it does
not authorize or prove provider activation.

The web package retains its canonical and legacy documents for Public
composition. The hosting packager includes only the three canonical documents,
rewrites browser assets and illustrations under `/connect`, and rejects
unexpected inputs and missing resource edges before replacing generated output.
The Worker separately retains only the exact legacy redirect map described
above; it does not render or package legacy `/hosted` documents or assets.
Its build profile uses `VITE_HOSTED_API_ORIGIN` and `API_ORIGIN` set to
`https://app.tryapparent.com/connect`, with `SITE_ORIGIN` set to
`https://app.tryapparent.com`. Public's legacy producer profile is unchanged
until cutover.

Hosting source inputs include `connect/tooling/hosting/`, `connect/service/`
and the full browser build closure: `connect/web/{package.json,bun.lock,tsconfig.json,
index.html,vite.config.ts,src/**,scripts/**}`, `shared/ui/{src/**,
scripts/check-theme.py}`, `shared/analytics/{config.ts,browser.ts}`, and the five
approved files `public/site/public/assets/{hosted/{source-pond-v4,agent-pond-v4,
agent-reading-v4}.png,fonts/HostGrotesk-Variable.ttf,brand/apparent-wordmark.svg}`.
The parent CI entrypoints are `npm ci`, `npm run typecheck`, `npm run build`, and
`npm test` from `connect/tooling/hosting`; the common CI lane calls
`npm run verify:artifact`, which installs locked web dependencies, prepares browsers,
runs the browser suite and the real local Worker checks, and builds the final
asset/runtime payload. PR verification isolates its synthetic analytics environment
from this publishable graph by disabling analytics there; production main keeps
its configured, fingerprinted analytics values and rejects fixture contamination.
Install Connect web with
`bun install --frozen-lockfile` from `connect/web` first. Build output is
`connect/tooling/hosting/.output/dist`: three `index.html` documents, one hashed
JS/CSS pair under `connect/assets/`, the font and wordmark under that same
namespace, and three `/connect/hosted/*.png` images (10 files total).
Wrangler's static asset binding consumes this directory. `.output/worker.js` bundles
the locked Connect service implementation and `.output/wrangler.json` is its route-free deployment config;
publication consumes these verified bytes with `--no-bundle` rather than building
source. Rewritten assets revalidate rather than using their pre-rewrite filenames
as immutable content identities. This package does not change delivery artifact schema, route
inventory, or producer names and does not require Cargo or rustc.

`npm test` starts Wrangler's local Workers runtime against the packaged static
asset directory and exercises the six canonical route/slash forms, query
preservation, resource fetches, service-owned API/viewer dispatch, HEAD/GET,
method denial, unknown and encoded paths, and privacy/security headers. A
handler boundary test checks sensitive request headers and device-code queries
are removed before the static asset binding; the browser-visible query is retained.
The runtime test hydrates the rewritten package in Chromium with the synthetic
session endpoint intercepted, checking same-origin sign-in destinations and
device-code preservation. Run the existing browser
suite separately from `connect/web` with a private evidence directory, for
example `CONNECT_EVIDENCE=/tmp/connect-web-hosting-evidence npm test`; it covers
synthetic account/device/share states, analytics, no-JS loading HTML and
artifact/resource closure. Neither lane logs in or accesses private recordings.
The local Worker run does not establish zone route activation, live cookies,
OAuth, provider configuration, or deployment.

Legacy `https://hosted.tryapparent.com/s/<capability>` HTML document requests (GET/HEAD) redirect temporarily to `https://app.tryapparent.com/connect/s/<capability>`, with no-store/no-referrer/noindex and query preservation. Metadata and file endpoints under the old link remain service requests for existing clients, not redirects. Existence and authorization are checked by the canonical destination; a redirect does not prove that a recording exists or is ready.

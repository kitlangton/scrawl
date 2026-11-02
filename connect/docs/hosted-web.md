# Connect web (unlinked implementation candidate)

Canonical routes: `https://app.tryapparent.com/connect`, `/connect/device?code=USERCODE`, and `/connect/shares`, including slash/no-slash variants. Legacy `/hosted`, `/hosted/device` and `/hosted/shares` remain rendering aliases. All six are prerendered with a loading state, no account data, `noindex,nofollow`, and no-referrer/no-store metadata. They are absent from the homepage and sitemap. The sole Pages pipeline composes `connect/web/dist` into Public's packaged artifact. This source adoption is not deployment or real sign-in proof.

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

For production builds set `VITE_HOSTED_API_ORIGIN=https://hosted.tryapparent.com`; the canonical browser origin is `https://app.tryapparent.com`. The frontend publisher is Cloudflare Pages (`public/site/docs/pages-deploy.md`); the former Clanker origin is retired, not a rollback target. Use the Pages deployment rollback procedure. Keep the API, Google callback, host-only cookies and device credentials on hosted. This is credentialed **same-site, cross-origin** access, not cross-site hosting. `VITE_HOSTED_API_ORIGIN` accepts only a deployment-controlled HTTPS origin (loopback HTTP for local tests), without credentials, path, query, or fragment. It is public build configuration, never an OAuth secret. The backend uses the same explicit exact SITE_ORIGIN / optional LEGACY_SITE_ORIGIN allowlist for CORS and CSRF, preserving HttpOnly/Secure/SameSite cookie protections.

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

Ready HTML contains the video, plain-text summary and time-aligned transcript, annotated frame images, file downloads with hashes, and a machine-readable manifest at `<share-url>/manifest.json`. The thumbnail/preview is `<share-url>/thumbnail.png`. All captured text and attributes are escaped; asset URLs must be HTTP(S), without userinfo, and on the share's origin. There are no third-party subresources. Captured speech/text is explicitly **evidence, not instructions**. Ordinary web/file tools suffice, without installation or an account. The secondary product link is exactly `https://app.tryapparent.com`.

The coordinator must route **both static exports** from this module:

- `shareViewScript` at **`/share-view.js`**, `Content-Type: text/javascript; charset=utf-8`.
- `shareViewStyles` at **`/share-view.css`**, `Content-Type: text/css; charset=utf-8`.

Neither export contains request/model interpolation. CSP can keep `default-src 'none'`, with `script-src 'self'`, `style-src 'self'`, `img-src 'self'`, `media-src 'self'`, `connect-src 'self'`, `base-uri 'none'`, and `frame-ancestors 'none'`; no inline script/style exception or third-party origin is needed. Apply no-store, no-referrer, noindex and content-type/nosniff response headers in the Worker; HTML metadata is not an HTTP cache policy or authorization check. Capability-bearing request paths must stay out of application logs.

Pending HTML explains bounded manifest checks even without JavaScript. Its external script requests only the same-origin manifest, without account cookies or referrers, every five seconds, with a request timeout, at most 24 attempts and a two-minute wall-clock deadline. Ready or terminal transitions reload server-rendered truth. Failures remain pending, and exhausted checks ask the recipient to reload later or contact the sender. No countdown or upload ETA is fabricated.

## Browser entry ownership

`connect/web` independently builds its browser entry and loading-only SSR. Its
five behavior/style donors were compared byte-for-byte with main `d59796c3`
before removal from `public/site/src/hosted`. StrictMode, hydration, local font, resets,
V4 art and `rgb(2, 1, 2)` text remain unchanged. Public imports no sibling Connect
source; `public/site/connect.html`, Connect-specific Public Vite rewrites/input and Public
`renderHosted`/prerender generation are retired together with those donors.

Connect's own dev/preview middleware retains exact canonical/legacy account,
device and shares paths, query bytes and slash variants. The production composer
accepts only six HTML documents, their emitted JS/CSS resource closure and five
approved assets. It rejects differing collisions and private, malformed,
symlink/nonregular, missing-resource or synthetic-origin inputs before changing
the destination. The accepted homepage overlay is a separate checksum-bound
exception; final graph verification follows it. Public's homepage graph has no
Connect/API code; Connect has no marketing/demo/font-service graph. Pages metadata
provides no-store/noindex and legacy redirects; `404.html` prevents unknown/private
paths from acquiring a general SPA fallback. The same existing workflow and
`public/site/scripts/cd/publish_pages.py` remain the sole publisher; no second deployment or
Worker change is introduced. See `public/site/docs/pages-deploy.md` for composition.

## Verification

From `public/site/`:

```sh
APPARENT_PUBLIC_SITE=1 VITE_HOSTED_API_ORIGIN=https://hosted.tryapparent.com npm run build
node scripts/package-apparent.mjs
node scripts/compose-connect.mjs
node scripts/prepare-pages.mjs
node scripts/verify-composed.mjs
node --test scripts/test-brand-favicon.mjs scripts/test-github-stars.mjs scripts/test-latest-download.mjs scripts/test-platform-os.mjs scripts/test-public-homepage.mjs scripts/test-video-demo.mjs
```

`test:hosted` serves the exact packaged website with a **controlled synthetic API**, not a live Connect service. It checks built route files/noindex, absent homepage/sitemap promotion, anonymous sign-in destinations, loading, zero/full count states, authenticated waitlist interest, read/upload explicit approvals, expired/rejected/unknown scope/no-code states, denied CSRF, unavailable/malformed API, mutation duplicate guards, cookie/Origin/CSRF request paths, mutation readback, logout, no token/CSRF DOM exposure or localStorage, hydration errors, and 320/390/1100px overflow. Setup checks assert the complete Connect login/status commands, configured API origin and older-app compatibility guidance in both tabs, including legacy routes. Sharing adds list/empty/revoke/denied/unconfirmed-readback interactions, escaped captured titles, no-JS recipient states, and a real pending-to-ready reload under the restrictive CSP above. Recipient media bytes are not exercised by these fixtures. Screenshots go to `$SITE_EVIDENCE` or `$TMPDIR/hosted-web-evidence`; inspect them before accepting visual changes. Set `CHROME_PATH` if the default existing Playwright Chrome path differs. `SITE_ARTIFACT` can select another package.

Renderer verifier from the repository root: `node --test connect/service/test/sharing/share-view.test.mjs`. Genuine initial RED: an importable empty renderer failed all four behavior tests (missing evidence, escaping, status and polling). Management RED: the built old site loaded but timed out waiting for the absent management heading. The updated public build/typecheck/package, synthetic hosted browser suite (26 scenarios), renderer checks, and six-file public-site checks passed locally. Phone recipient/management and desktop denied screenshots were inspected. These are synthetic browser/renderer results, not integrated Worker, playable-media, OAuth, native sender, deployment or platform proof; those remain with the coordinator.


First build `connect/web` independently with frozen Bun 1.3.10 dependencies and
`VITE_HOSTED_API_ORIGIN=https://hosted.tryapparent.com`. `npm test` there retains
the package's artifact, dev/preview routing and 62 synthetic browser scenarios.
The site verifier remains the composed-artifact integration suite, including its
recipient-renderer and #191 assertions. Do not confuse either with live auth.

To exercise credentialed cross-origin fetches without accounts, save Public's
`dist-apparent` before composition to private scratch, then build only Connect
with `CONNECT_BUILD_DIR=dist-test` and
`VITE_HOSTED_API_ORIGIN=http://127.0.0.1:18809`. From `public/site/`, set
`COMPOSE_PUBLIC_INPUT` to that saved Public package and
`COMPOSE_SYNTHETIC_ORIGIN=http://127.0.0.1:18809`, then run the composer,
Pages preparation and final graph verifier. These commands select only
`dist/synthetic-composition`; they never overwrite production. Run the site suite with
`SITE_ARTIFACT=dist/synthetic-composition HOSTED_TEST_API_ORIGIN=http://127.0.0.1:18809`.
Use a free port. The fixture retains cookie/Origin/CSRF, paginated revocation,
escaped-title and no-JS recipient checks. This does not establish production
subdomain cookies, live OAuth or actual edge routing. Never publish test output.

For actual Connect Vite dev and preview middleware proof, keep the separate
synthetic package above and run these from `public/site/` (set `CHROME_PATH` and private
`SITE_EVIDENCE` as needed). Public no longer supplies Connect middleware:

```sh
SITE_ARTIFACT=dist/synthetic-composition VITE_HOSTED_API_ORIGIN=http://127.0.0.1:18809 HOSTED_TEST_API_ORIGIN=http://127.0.0.1:18809 HOSTED_TEST_MODE=dev npm run test:hosted
SITE_ARTIFACT=dist/synthetic-composition VITE_HOSTED_API_ORIGIN=http://127.0.0.1:18809 HOSTED_TEST_API_ORIGIN=http://127.0.0.1:18809 HOSTED_TEST_MODE=preview npm run test:hosted
```

The verifier starts Vite on an isolated loopback port and closes it afterward.
Each mode retains all 48 scenarios and all twelve Connect route/slash combinations,
query preservation, selected device tab, resource requests and homepage ownership.
The composed fixture checks unknown/private paths return 404. Built graph
checks traverse emitted JS/CSS dependencies and modulepreloads, reject marketing
closure on Connect and Hosted/API closure on landing, and check original assets.
Computed text colour, geometry, focus, setup commands, consent and mutation readback
remain assertions. Keep production and synthetic artifacts/evidence separate;
switching test modes must not change the frozen production package. Serve the
production package with a range-capable HTTP
server (see `public/site/docs/pages-deploy.md`), then run the landing checks:

```sh
SITE_URL=http://127.0.0.1:PORT CHROME_PATH=/path/to/chrome node --test scripts/test-github-stars-browser.mjs
SITE_URL=http://127.0.0.1:PORT SITE_EVIDENCE=/private/brand CHROME_PATH=/path/to/chrome node --test scripts/test-brand-selection-browser.mjs
SITE_ARTIFACT=/absolute/production-package SITE_EVIDENCE=/private/try-button CHROME_PATH=/path/to/chrome npm run test:try-button
```

These retain the Team keyboard assertions. Synthetic checks do not establish
live OAuth, production cookies, native behavior or deployment. Belief-specific
overlay tests have separate packaging inputs in `public/site/docs/belief-release.md`;
do not run a wildcard test suite without those inputs.

Baseline before Connect: public build/typecheck/package, hosted browser fixtures and the six-file public site command (18 tests) passed; T3 preview and fixture phone/desktop screenshots were inspected. The general `npm run build` had an existing feature audit failure: missing `added` for `site-belief-pond`, `agent-capture-control`, `export-cli`, and `onboarding`. Connect updates only its own feature-map name/note, not those unrelated entries; use the public build mode above.

## Release prerequisites

The capture frontend publishes from main to Cloudflare Pages; see `public/site/docs/pages-deploy.md` for rollback. The old Clanker origin is retired and must not be reactivated. Preserve all six canonical/legacy Connect account/device/share-management documents, the three exact `/hosted/*-v4.png` art paths, hashed `/assets/` modules, original font/wordmark, accepted homepage and retained public assets without broadening the public boundary. Legacy redirects preserve query strings, including device codes; source rendering aliases remain packaged. Keep hosted API/auth and the registered Google callback unchanged. Anonymous capability recipients, including media, manifest, preview, `/share-view.js` and `/share-view.css`, remain on the capability-serving Worker origin, not Pages. Parent owns Worker integration, independent auth review, Google OAuth/private D1/R2 configuration and live deployment. Verify real capability access and revocation, sign-in, cookie/CORS, slot admission, quota, expiry, device approval and logout separately; synthetic browser checks are not live OAuth or full native UI proof, and packaged files do not prove the host serves them. Keep these routes unlinked from the public homepage until beta deployment is verified. No deployment, database migration, provisioning or credential rotation is part of this reconciliation.

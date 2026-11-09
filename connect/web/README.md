# Connect browser product — migration staging

Independent React/Vite browser + loading-only SSR package. It owns account and
waitlist screens, Google sign-in entry, scope-specific device consent and readback,
and the existing experimental shared-link list/revocation surface from #191.
The existing combined website is unchanged and remains operational.
Package scaffolding and browser fixtures selectively reuse staging #200 head
`6123548ed5616683725a6723337fa147ab3418d9`; current main source, not staging,
is the behavior authority. The lock was regenerated with Bun 1.3.10 because the
staging lock's newer format is not readable by the shared pinned toolchain.

## Build and recovery

From `connect/web` (Node 22.12+ and Bun 1.3.10):

```sh
bun install --frozen-lockfile
npm run build
CONNECT_EVIDENCE="$TMPDIR/connect-browser-check" npm test
```

`npm test` rebuilds the production artifact, checks its shape/privacy, then builds
an isolated `dist-test` and runs the inherited browser scenarios against a labelled
synthetic API. `CHROME_PATH` selects Chromium (default is the repository's
`/home/simon/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome`). No live
OAuth, real account, backend deployment, native app or production routing is proved.
If Bun is not on PATH, `npx --yes bun@1.3.10 install --frozen-lockfile` works.
Do not generate an npm lock. The ordinary generated Bun lock stays below 2,000 lines.

`npm run dev` serves the browser template; first run `node scripts/assets.mjs` if
assets have not been staged. Build failures are safe to retry; Vite clears the
chosen output and prerender recreates all six routes. `dist-ssr` is transient.
Never substitute `dist-test` for `dist`; it embeds a loopback API origin.

For a standalone checkout, copy only this package to a directory outside the
repository and supply `CONNECT_ASSET_SOURCE=/absolute/approved-assets` containing
the five files below in their canonical source groups (`hosted/`, `fonts/`, and
`brand/`). Run the same frozen install, `npm run typecheck`, and
`npm test` there. No `public/site/node_modules`, site source, root manifest, native helper
or backend implementation is required. The default asset source is only a build
convenience, not an implementation dependency.

## Thin integration contract

- Build cwd: `connect/web`. Output: `connect/web/dist/`.
- Merge only the six route directories (`connect/`, `connect/device/`,
  `connect/shares/`, `hosted/`, `hosted/device/`, `hosted/shares/`), hashed `assets/`, and the five allowlisted public assets into
  the composed website. There is deliberately **no root `index.html` or sitemap**.
  Do not overwrite the marketing root or admit Connect to public navigation.
- Serve `/connect`, `/connect/`, `/connect/device`, `/connect/device/`,
  `/connect/shares`, `/connect/shares/`, and matching `/hosted` aliases with
  their matching HTML, retaining the full query including `?code=...`.
  Do not redirect away or strip approval codes. The origin router must return 404
  for unknown paths rather than a general SPA fallback.
- All HTML is `noindex, nofollow`, loading-only, with no account or token props.
  The browser fetches private state after hydration. Response-layer no-store and
  X-Robots-Tag may be added by the composition owner; no account data enters SSR.
- Production browser origin stays `https://app.tryapparent.com`. Production API,
  Google auth and registered callback stay `https://hosted.tryapparent.com`.
  `VITE_HOSTED_API_ORIGIN` is a build override, defaulting to that existing API.
  Deploy topology must retain the exact credentialed CORS/CSRF allowlist and
  host-only Secure/HttpOnly/SameSite=Lax cookie contract from `connect/docs/hosted-api.md`.
- Five approved assets are copied at build time from canonical `public/site/public/assets`:
  `/hosted/source-pond-v4.png`, `/hosted/agent-pond-v4.png`,
  `/hosted/agent-reading-v4.png`, `/HostGrotesk-Variable.ttf`,
  `/apparent-wordmark.svg`. Original URLs and bytes are checked. No other site
  source, marketing CSS, navigation, manifest or external font request is needed.
- `src/base.css` contains only the inherited box model, font, smoothing, link,
  pointer and selection rules used by this surface. `base.css`, `hosted.css`,
  `Hosted.tsx`, `SharedLinks.tsx` and `api.ts` are byte-identical copies of
  current-main donors at `d59796c3` (after #191). Sharing retains #191's
  experiment status; this synchronization introduces no new UI behavior.

## Removal gate and rollback

The duplicate is intentional **migration staging**, not two permanent owners.
This is checklist **5b** only. Production adoption **5c** remains blocked on Pages
owner `99ea5290-aecc-4bf0-8aca-3200f62f5e5c` clearance. `public/site/` still owns the live
build and publisher; production build separation is not complete.
Integration must wire builds/artifact composition, route aliases, deployment and
CI; exercise the exact composed artifact against the same browser contracts;
verify live OAuth/callback/CORS/cookie behavior in separately authorized staging;
then remove `public/site/src/hosted` and its route/SSR/prerender wiring. Preserve approved
visuals and explicit consent. Update the owner documentation in that integration
change, not this exclusive-ownership slice. Roll back by restoring legacy route
composition; no API, cookie, schema or native credential migration is required.

## Verification scope

`test/artifact.test.mjs` rejects marketing/root/discovery output, verifies all
six private loading pages and exact approved asset bytes, and checks owner-local
runtime imports/storage/logging. `test/browser.mjs` exercises packaged output,
slash aliases, query-code preservation, keyboard/modal/layout and auth behavior:
separate upload consent/read scope, CSRF + cookies, duplicate-write guards,
readback mismatch, expiry, malformed responses, waitlist, logout and no token DOM
or storage leakage. All 32 inherited account/device scenarios remain selected.
Management checks cover pagination and full-list mutation readback, malformed or
repeated cursors, duplicate IDs, safe unexpired HTTPS links, escaped titles,
credentialed CSRF DELETE/duplicate guards, anonymous fixed return, stale loads,
failure/reload states and every status. Real dev and preview hydration checks
exercise all 24 route/slash variants with queries intact. Fixture screenshots
and receipts are synthetic evidence only.
Production artifact inspection and cross-origin checks are separate gates.

Independent review belongs to the coordinator: this worker does not nest delegates.
## Shared appearance

System light/dark appearance now comes from `shared/ui/src/tokens.css`, also
used by Desktop, recording recipients and Public. Account-local colors map to
semantic tokens; live system changes require no reload. This authorized theme
adoption supersedes the historical behavior-identical CSS extraction note above;
it does not change route composition, consent, API behavior or asset ownership.
A standalone copy must include the `shared/ui` directory at the same relative
path (including the theme guard and its baseline). The guard works without Git. See `shared/ui/README.md` for regression checks and native evidence limits.

# Connect web design V4

The Connect React UI implements the selected V4 paper-sheet composition. It uses the existing Host Grotesk font and Apparent wordmark unchanged, with revisioned illustrations in `public/hosted/`. Canonical routes are `/connect`, `/connect/device` and `/connect/shares`; `/hosted` counterparts remain rendering aliases. Visible headings are “Upload new captures” and “Read your captures”; read grants still cover current and future uploads. The API remains `https://hosted.tryapparent.com`, independently configured with `VITE_HOSTED_API_ORIGIN`. There are no prototype controls, native capsule, simulated clipboard, native launch, or locally invented connection states.

## Supported mapping

| Surface / verified input | Web behavior |
| --- | --- |
| `/connect` (or `/hosted`), anonymous | This computer / Upload new captures / Connect. Your agent / Read your captures / Review access. Either action opens sign-in; Google authentication alone grants neither permission. |
| `/connect` (or `/hosted`), active account | Same initial composition, not an inferred connected state. Connect / Review access opens terminal setup instructions, not native launch or permission mutation. |
| Setup, upload | `apparent connect login --server <API origin> --scope upload --consent-upload`, followed by `apparent connect status` and `apparent connect status --account`. Explicit consent; new captures only; add/read/delete scope. |
| Setup, read | `apparent connect login --server <API origin> --scope read --no-browser`. Separate approval of all current and future Connect uploads; no upload/delete. |
| Installed app compatibility | Both setup tabs explain that `apparent connect` requires an updated app. Older apps can use `apparent cloud`, retained as a compatibility alias; no installation or version is inferred by the website. |
| API origin | Configured `VITE_HOSTED_API_ORIGIN` is used in commands. When absent, a clearly marked placeholder must be replaced with the deployment's actual API origin; the website origin is not assumed to be the API. |
| `/connect/device?code=…` (or `/hosted/device?code=…`), signed out | Sign-in keeps only the fixed canonical approval route and its code. No visitor-supplied return URL is forwarded. |
| Loading | Checking your account. No approval action. |
| Active account + verified pending device response | The server's immutable `upload` / `read` scope chooses the tab. Tabs cannot change the request. Scope and sensitive categories are visible before explicit approval. Upload requires a consent checkbox as well as Allow uploads; read requires Allow agent read access. |
| Approval | One guarded POST with CSRF, then GET device readback. Both approved status and the original scope must match. Only then show Approval granted; CLI must finish. This does not establish local upload settings, token receipt, or a completed read. |
| Already approved | Same truthful approval state, no second mutation. |
| Rejected / expired | Nothing approved; start a new CLI request. Pending expiry is checked while the page is open and again immediately before approval. |
| Invalid scope / bad session data / service or CSRF error / unconfirmed readback | Not confirmed, specific safe error, Reload account. No success or inferred native state. |
| Waitlist | Beta capacity and optional $5/month interest, no billing. Explicit submit, duplicate-write guard and session readback. |
| Log out | Browser session only; POST then anonymous session readback. Does not revoke independent device grants. |
| Separate lock control | Describes independent read approval and limits. It is not a claim about an agent credential's effective state. |

Account tabs support arrow keys, Home/End and roving focus; optional detail uses native disclosures. The setup dialog uses native modal focus containment, Escape and focus return. No request writes on page load, tab switch, disclosure or setup. V4 approval uses the existing API contract; `api.ts` generates fixed canonical Connect return destinations. Server validation, credential handling and CSRF boundaries still apply. The separate sharing implementation adds list/revoke API methods without changing those approval boundaries.

Upload and read permission disclosures retain screen images, recording, voice, transcript and context; 20-account beta, 500,000,000-byte account cap with oldest-first eviction, seven-day maximum retention, 90,000,000-byte per-file rejection of the whole bundle, and fixed 30-day grants with manual reapproval and no website renewal.

## Deliberately deferred

The V4 account-design slice does not implement a native connected/upload-enabled display, capsule, per-capture upload status, inspection/Paste, automatic browser return, retries, expiry/oversize recovery, or connection/grant management, disconnect and revocation UI. This does not exclude the separate single-capture sharing implementation: signed-in beta accounts can open **Manage shared links** at `/connect/shares` (legacy `/hosted/shares`) to list and stop sharing individual captures. That action revokes a public capability, not a device grant, and does not delete the private recording. The browser API cannot observe a computer's settings or credential. Local capture/Paste and CLI recovery remain governed by [the client contract](../../../connect/docs/hosted-client.md). A web approval is not native integration proof.

## Verification and delivery

From `public/site/`:

```sh
APPARENT_PUBLIC_SITE=1 npm run build
node scripts/site-build/package-apparent.mjs
SITE_EVIDENCE="$TMPDIR/hosted-v4-evidence" npm run test:hosted
```

The public build intentionally omits the private feature-map route; the ordinary build has a separately recorded pre-existing feature-map validation failure unrelated to this slice. Do not repair unrelated feature entries to validate this UI.

The browser script serves the exact `dist-apparent` artifact by default against a controlled synthetic API. It compares packaged art, font and wordmark bytes with public source assets, verifies SSR privacy and noindex, scopes, explicit consent, CSRF, duplicate-write protection, approval readback failures, loading/errors, expiry, waitlist, logout, tab keyboard behavior, disclosures, modal focus return and loaded illustrations. Layouts cover 1440, 1280, 390 and 320 CSS pixels plus 640×450 (the layout equivalent of a 1280×900 viewport at 200%, not actual browser zoom). Evidence stays outside git and includes screenshots and an asset-hash receipt.

These checks are synthetic, not live OAuth, service, native platform or deployed-site proof. Parent review and deployment are separate steps; this implementation does not publish a site or backend.

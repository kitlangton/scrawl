# Frog theatre footer

The accepted footer is concept 04: paper above, theatre below. The generated artwork is the original owner-selected frog theatre, encoded as `public/site/public/assets/pond/footer-theatre.webp` (1672×941, quality 90). It contains no capture screenshots or personal data. Private studies and feedback recordings must never be packaged.

`public/site/src/footer/Footer.tsx` and `src/footer/` are the canonical implementation. Desktop keeps the 688px scene but combines the wordmark, rotating credit and GitHub link in one 84px base row, removing the separate credit row. Narrow layouts keep a compact two-row base without squeezing or clipping the full phrases. The four phrases include their prepositions. Hover/focus pause rotation, clicking toggles pause, hidden documents stop timers and reduced motion disables rotation. The custom element removes listeners and pending animation work when detached.

The primary button first attempts `apparent://open`. Before that navigation, its text changes to “Not installed yet? No worries—click again to download ↗”. The next click activates the existing hero download link, retaining its platform/CPU selection, latest-release lookup, busy guard and release-list fallback. Without a supported hero download, it opens the Mac/Linux release chooser. There is no installation detection, timer or automatic download. The label remains in its download state until the page reloads, including after a custom-element reconnect. Hero and footer share `public/site/src/cta.css`, using the footer’s rounded style.

Opening the app requires native URL-handler support from #159 / #162 and an app release with that feature enabled (`nativeLaunch` is off by default in that candidate). Older installs or a rejected browser prompt leave the explicit download action available; a website cannot verify that launch succeeded. Website tests prove the requested URL and download routing, not native launch on macOS or Linux. Opening must not start capture, upload or send feedback. The LinkedIn chooser retains its native dialog, Escape, focus containment and focus return.

## Verification and publication

Build with `APPARENT_PUBLIC_SITE=1 npm run build`, package with `node scripts/site-build/package-apparent.mjs`, and run the `node:test` scripts (not standalone browser programs without their environment). `SITE_ARTIFACT=/absolute/artifact SITE_EVIDENCE=/private/evidence node scripts/commerce/test-footer-browser.mjs` exercises desktop/mobile, all phrases, pause/resume, reduced motion, dialog focus and detach cleanup. `SITE_URL=https://app.tryapparent.com/` selects live acceptance instead. Evidence stays outside public assets.

`SITE_ARTIFACT=/absolute/artifact SITE_EVIDENCE=/private/evidence npm run test:try-button` exercises the two-click flow at 320/390/768/1280px, with mocked Mac ARM and Linux x86/ARM browser hints, unknown CPUs, unsupported platforms, failed release API, keyboard activation and custom-element reconnect. It observes the exact protocol navigation and asserts no release request after the first click, then downloads only a clearly labeled text fixture on the second. It never opens a real app or fetches a real installer.

The headless test uses real mouse/keyboard input for launch and a DOM-dispatched follow-up click for download routing: browser-owned external-protocol prompts can block later automated input. It does not certify native consent/dismissal. Use a desktop browser to verify both physical clicks and the native prompt before release.

The packaging baseline must be a real directory, not a symlink; use `readlink -f` to inspect the release target first. A symlink baseline is rejected before any candidate write because copying the link itself would otherwise let later writes reach the live release. Packaging tests first copy the resolved baseline to a disposable directory, then test symlink rejection, idempotence and conflicts against that copy. Set `ACORN_MODULE` to an installed Acorn package if it is not available through normal module resolution.

Deploy only a scoped overlay: `node scripts/commerce/package-footer-release.mjs BASELINE NEW_CANDIDATE EXTERNAL_MANIFEST`. It uses the same markup, footer/shared-CTA CSS and behavior as the source build, replaces the SSR and React footer, and asserts every unrelated HTML byte and pre-existing asset is preserved. Shared CTA CSS intentionally also rounds the hero download; its existing JavaScript still selects the download. It reuses byte-identical immutable files and refuses conflicting ones. `SITE_BASELINE=/absolute/active-release node --test scripts/commerce/test-footer-packaging.mjs` covers initial/repeated publication and conflict refusal. Run both browser programs against the packaged overlay as well as the source artifact before publication.

Production activation and rollback now use only Pages (`pages-deploy.md`). The
Clanker activator, serving block and release link are retired; do not recreate
them. Reconcile the tested overlay into `published-homepage/` and its checksum
contract before main-branch publication. Verify the canonical homepage and new
assets against the accepted package afterward. No Caddy, appcast, installer or
unrelated route change is needed.

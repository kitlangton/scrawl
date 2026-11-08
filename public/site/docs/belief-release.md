# Scoped belief release

The public host serves an approved movie-demo artifact. The movie demo is now also on `main`; the canonical belief component lives in `public/site/src/components/home/BeliefPond.tsx` and `src/belief/`, and `public/site/public/assets/pond/belief/` contains only the six approved sprites. Keep this release scoped to the belief instead of replacing unrelated production assets with a whole build.

`package-belief-release.mjs BASELINE NEW_CANDIDATE EXTERNAL_MANIFEST` makes an isolated overlay on a frozen production release. Node 22's type stripper emits the standalone behavior from the same source used by the normal build. `ACORN_MODULE` may point at an existing Acorn installation. The script replaces only the belief SSR markup and its parsed React render node, adds hashed behavior/CSS/root JS and the six sprites, and asserts all old assets and all other HTML bytes remain unchanged. It refuses to overwrite a candidate or put evidence inside the public package. No prototype lead-in/tail, review copy, recordings or local paths ship.

Run the normal `APPARENT_PUBLIC_SITE=1 npm run build` as source proof, but publish only the tested overlay. Browser regression: `SITE_ARTIFACT=/absolute/candidate SITE_EVIDENCE=/private/evidence PLAYWRIGHT_MODULE=/existing/playwright-core/index.mjs node scripts/test-belief-pond.mjs`. This covers desktop and 320/375/430px mobile widths, Me too, keyboard/reset, the leap and pose transition, both lily pads, reduced motion, exact sequential scroll progression and reversal, no drift at rest and hydration errors. Set `SITE_URL=https://app.tryapparent.com/` to run the same checks against production. Runtime cleanup cancels listeners/timers/animations; viewport changes during flight settle directly into the responsive hug.

Mobile uses 32px text gutters and an inset scene so both button labels, frogs and pads stay at least 24px from either screen edge. The scene starts 16px after the heading, is 180px tall, and raises its frog baseline; the regression caps the heading-to-frog gap at 90px. Desktop geometry stays unchanged. The section retains its viewport-height scroll runway below the compact group so both marks can finish drawing before the end of the page.

The packager supports a repeat release: it balances the nested study section, replaces old belief stylesheet/module references instead of stacking them, reuses byte-identical immutable assets, and refuses conflicting bytes. `SITE_BASELINE=/absolute/initial-release node scripts/test-belief-packaging.mjs` tests initial and repeat overlays plus conflict refusal. `python scripts/test-belief-publication.py` exercises activation, guarded rollback and publisher contention only in a temporary filesystem.

The Clanker activator and symlink rollback are retired; their temporary-filesystem
tests above are historical protocol tests, not production instructions. Do not
run `activate-belief-release.py` on Clanker or recreate its deleted release link.
Reconcile any verified overlay into `published-homepage/` and its checksum
contract, then use main-branch Pages delivery and Pages rollback as documented in
`pages-deploy.md`. Verify public hashes and desktop/mobile interactions after
publication. Team absence belongs only to the historical belief-only acceptance;
do not remove subsequently approved Team content to satisfy that old scenario.

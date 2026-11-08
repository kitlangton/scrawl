# Public Apparent site: Pages; Clanker origin retired

Production publication now uses the `apparent-capture` Cloudflare Pages project
through main-branch GitHub Actions. See [`pages-deploy.md`](pages-deploy.md).
The old capture-site serving block, release symlink and historical release
directories have been removed after canonical acceptance. Do not recreate them
or restore the old DNS A record. `public/site/scripts/cd/publish_site.py` refuses production
publication; Actions does not use SSH or the retired release tree. Recovery stays
inside Pages, not Clanker.

For checked main-branch delivery, activation gates, deployment credentials and
Pages rollback, see [`../../../enablement/docs/continuous-delivery.md`](../../../enablement/docs/continuous-delivery.md).
Historical publishers' cooperative lock file remains to avoid splitting lock
ownership. It is not an active publication path.

Before cutover, `app.tryapparent.com` reached `presentations-caddy-1` through
Cloudflare. That container also serves unrelated sites. It and its volumes,
`/opt/presentations/Caddyfile`, other release directories and document-product
deployment services remain unchanged except for removing the capture site's
serving block. Never remove the shared proxy as capture-site cleanup.

The former manual publication and symlink-rollback procedure is intentionally
removed. The accepted public snapshot is now source-controlled; follow
`pages-deploy.md` to reconcile an overlay without losing approved homepage bytes.

## Homepage marker guard

`scripts/site-build/package-apparent.mjs` uses `scripts/site-build/package-page-text.mjs` as a parser-only stale-build
marker guard, not an HTML sanitizer or proof of CSS/layout visibility. Its
intended content boundary is parsed body text, excluding `script`, `style`,
`template`, `noscript`, and `iframe` subtrees, any subtree with a `hidden`
attribute (regardless of its value), and any subtree with a literal inline
`display: none` declaration (case-insensitive, with optional `!important`).
Descendants cannot supply markers once their ancestor is excluded. This is a
conservative exclusion even if a later declaration would override it.

The guard does not evaluate the CSS cascade, CSS escapes or variables, external
CSS, layout, or dynamic state. Other ways of concealing content remain outside
its scope; browser visibility still needs browser verification. Normal body
text across inline elements and decoded entities can supply markers. Extraction
does not rewrite the packaged HTML. Run `node --test scripts/commerce/test-package-text.mjs`
to check acceptance and rejection through the real packager, including preservation
of the previous artifact when marker validation rejects a build.

## Recorded ghost demo

`public/site/src/demo/core/VideoDemo.tsx` replaces the public scripted animation with the approved
`public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4`. Keep this file byte-identical to the
approved export. Its poster is a still from that same export. The movie has burned
subtitles, lasts 59.818667 seconds, and loads only after a playback or seek action.
Chapter times are final-export seconds: Start 0, Capture 3.733333, Sketch 15.433333,
Handoff 27.92, Result 48.7, Try it 55.7. Controls follow the real video clock.

The Pages package boundary includes only the approved movie, poster and public
assets. Preserve the accepted snapshot's combined markup and CSS when changing
it; copying images alone does not preserve a homepage. Record base and overlay
commits privately and update the source-controlled snapshot checksum contract.

After building and packaging, run the source and artifact Node tests with
`node --test scripts/brand/test-brand-favicon.mjs scripts/browser-proof/test-github-stars.mjs scripts/pages/test-latest-download.mjs scripts/pages/test-platform-os.mjs scripts/pages/test-public-homepage.mjs scripts/browser-proof/test-video-demo.mjs`.
The belief/footer overlay and browser scripts require separate `SITE_BASELINE` or
`SITE_ARTIFACT`/`SITE_EVIDENCE` inputs; the wildcard is not a standalone suite.
Against a
server that supports HTTP byte ranges, run
`node scripts/browser-proof/verify-video-demo-browser.mjs <URL> <private evidence directory>`.
`CHROME_PATH` can select an existing Chrome installation. The browser check covers
first-load chapter seeking, play/pause, replay, all chapters, sound, pointer and
keyboard seeking, absence of drawing controls and Shift-drag ink, natural ending, rapid switching, missing-media
feedback, and 390/320px layouts. It does not verify physical speakers or Safari.
Do not use Python's basic `http.server` to judge seeking: it ignores Range requests
and Chrome can clamp an early seek to zero. Require a live `206` response with
the correct `Content-Range`, compare the downloaded movie's SHA-256 with the
packaged source, and rerun the browser check on the public URL after cutover.

For layout changes, run `node scripts/browser-proof/verify-page-density-browser.mjs <URL>`
against the packaged page. It checks the download gap, shorter hero, uncropped
movie, and visible, reachable playback/chapter controls at seven desktop and
phone sizes, including a 600px-tall desktop. The movie width follows viewport
height so its controls do not fall below the screen. Keep the belief section's
scroll runway below its content: the marks must still complete before the page
ends, and the frog's jump must stay below the heading. Run the belief browser
regression from `belief-release.md` after changing those gaps.

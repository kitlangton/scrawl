# Homepage footer

The photo section leads into “We made this. Show us what you can do.”
`src/footer/` owns its markup, styling and lifecycle-scoped behavior.

Keep the playful “Try apparent” invitation: its personality is intentional. The
first activation requests `apparent://open` and changes the same button to
“Not installed yet? No worries—click again to download ↗”. The next activation uses
the hero's platform/CPU-aware download link, including its busy guard and release-list
fallback. No timer downloads anything. The site does not detect installation,
confirm native launch, start capture, upload or send feedback. Reconnecting the
footer preserves the explicit download state.

The native social dialog links directly to Olivia and Simon's LinkedIn profiles.
Escape and Close return focus to its opener. The bottom credit retains its
pauseable rotation; reduced motion disables it. Actions use square brand frames.

## Verification

Build and package the Public artifact, then run
`node --test scripts/pages/test-public-homepage.ts` and
`SITE_ARTIFACT=/absolute/artifact SITE_EVIDENCE=/private/evidence npm run test:try-button`.
The latter uses synthetic release responses and a text download fixture, with
Mac/Linux CPU variants, unknown/unsupported platforms, API failure, keyboard
activation and reconnect coverage. It proves web routing, not native launch or
native protocol-consent behavior. Evidence stays outside served content.

Publication remains subject to the current review hold. Normal main-branch Pages
CD and rollback are described in `pages-deploy.md`.

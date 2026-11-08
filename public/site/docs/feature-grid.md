# Homepage feature grid

The homepage places four animated explanations immediately after `VideoDemo` and before the belief section. Titles are black, bold, and left aligned. The layout is two columns on desktop and one on mobile.

`public/site/src/features/FeatureGrid.tsx` renders the section. The four isolated animation documents live in `public/site/public/assets/feature-demos/`; `scripts/site-build/package-apparent.mjs` explicitly includes them in the public artifact. Frames load near the viewport, pause offscreen, and share a visible pause/play control. Reduced motion starts paused. The documents accept play/pause messages only from their same-origin parent.

These are illustrative examples, not live recording or agent connections. Handoff uses sample words and placeholder paths; the visuals highlight image references, timestamped text, and interactions before an illustrative Send. Nothing is sent to an agent.

The whiteboard movie is a silent cropped excerpt (17.8–27.4 seconds) of the already approved `public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4`, excluding the webcam and terminal. The ghost graphics match that published demonstration. No private capture bundle or private audio is included.

Validate with the public-site build (`APPARENT_PUBLIC_SITE=1 npm run build`), `node scripts/site-build/package-apparent.mjs`, and the public homepage/video tests. This section changes no desktop behavior and includes no deployment step.

## Preserving the current live homepage

The live page can include approved overlays not yet reconciled into main. Build the isolated feature entry with `vite build --config vite.feature-grid.config.ts`, then run `node scripts/commerce/package-feature-grid.mjs BASELINE NEW_CANDIDATE PRIVATE_MANIFEST` from `public/site/`. It adds only the feature module/CSS and animation documents; existing files except the two new tags in `index.html` remain byte-identical. The entry mounts after the existing homepage hydrates.

Stage and inventory the candidate privately, then reconcile the tested overlay
into `published-homepage/` and its checksum contract for main-branch Pages
publication (`pages-deploy.md`). The Clanker Caddy block and release link are
retired; do not recreate them or run old symlink activators. Verify normal public
responses and browser behavior after publication. Never replace other approved
live changes with a whole-site build just to add this section. Recovery uses a
previously accepted Pages deployment, not a Clanker release.

## Approved homepage refinements

The fourth example is an illustrative agent walkthrough inside a chat interface. It expands at playback and captions explain the ghost eyes and button changes. The button highlight follows its DOM bounds. All examples retain parent pause/play handling; captions use the same 25px design-space size. No private reference movie, narration, source frames, or captures are shipped.

`homepage-revision.js` and `.css` retain the approved live footer/photo integration: handwritten names, removal of the two footer frog groups, and side-by-side LinkedIn names with the existing speech bubble. They are imported only by the isolated live overlay entry.

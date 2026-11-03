# Homepage feature grid

The homepage places four animated explanations immediately after `VideoDemo` and before the belief section. Titles are black, bold, and left aligned. Each feature has copy beside its animation on desktop and above it on mobile.

`public/site/src/features/FeatureGrid.tsx` renders the section. The four isolated animation documents live in `public/site/public/assets/feature-demos/`; `scripts/site-build/package-apparent.mjs` explicitly includes them in the public artifact. Frames load near the viewport, pause offscreen, and share a visible pause/play control. Reduced motion starts paused. The documents accept play/pause messages only from their same-origin parent.

These are illustrative examples, not live recording or agent connections. Handoff uses sample words and placeholder paths; the visuals highlight image references, timestamped text, and interactions before an illustrative Send. Nothing is sent to an agent.

The whiteboard movie is a silent cropped excerpt (17.8–27.4 seconds) of the already approved `public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4`, excluding the webcam and terminal. The ghost graphics match that published demonstration. No private capture bundle or private audio is included.

Validate with the public-site build (`APPARENT_PUBLIC_SITE=1 npm run build`), `node scripts/site-build/package-apparent.mjs`, and the public homepage/video tests. This section changes no desktop behavior and includes no deployment step.

## Source publication

The normal TypeScript/React build is the production input. Sequential feature
rows and accepted photo/footer presentation are source-owned; no secondary React
root or post-hydration DOM overlay is used. Merge reviewed changes and let the
verified main-branch Pages pipeline publish them (`pages-deploy.md`).

## Approved homepage refinements

The fourth example is an illustrative agent walkthrough inside a chat interface. It expands at playback and captions explain the ghost eyes and button changes. The button highlight follows its DOM bounds. All examples retain parent pause/play handling; captions use the same 25px design-space size. No private reference movie, narration, source frames, or captures are shipped.

`homepage-revision.css` retains the accepted sequential layout and handwritten
photo annotations. Photo markup lives in `PhotoStory.tsx`; social dialog markup
and interaction live in `src/footer/`. All are part of the normal source build.

# Homepage feature grid

The homepage places four animated explanations immediately after `VideoDemo` and before the belief section. Titles are black, bold, and left aligned. Each feature has copy beside its animation in alternating columns on desktop and above it on mobile.

`public/site/src/features/FeatureGrid.tsx` renders the section. The four isolated animation documents live in `public/site/public/assets/feature-demos/`; `scripts/site-build/package-apparent.ts` explicitly includes them in the public artifact. Desktop frames load near the viewport, pause offscreen, and share a visible pause/play control. Reduced motion starts paused. The documents accept play/pause messages only from their same-origin parent. They are decorative to assistive technology; complete headings and descriptions remain outside them. Phones show static, semantic examples with text at 14px or larger instead of scaling the desktop canvas. Hidden phone iframes stay paused.

These are illustrative examples, not live recording or agent connections. Handoff uses sample words and placeholder paths; the visuals highlight image references, timestamped text, and interactions before an illustrative Send. Nothing is sent to an agent.

The whiteboard movie is a silent cropped excerpt (17.8–27.4 seconds) of the already approved `public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4`, excluding the webcam and terminal. The ghost graphics match that published demonstration. No private capture bundle or private audio is included.

Validate with the public-site build (`APPARENT_PUBLIC_SITE=1 npm run build`), `node scripts/site-build/package-apparent.ts`, and the public homepage/video tests. This section changes no desktop behavior and includes no deployment step.

## Source publication

The normal TypeScript/React build is the production input. Sequential feature
rows and accepted photo/footer presentation are source-owned; no secondary React
root or post-hydration DOM overlay is used. Merge reviewed changes and let the
verified main-branch Pages pipeline publish them (`pages-deploy.md`).

## Approved homepage refinements

The fourth example is an illustrative agent walkthrough inside a chat interface. It expands at playback and captions explain the ghost eyes and button changes. The button highlight follows its DOM bounds. All examples retain parent pause/play handling; captions use the same 25px design-space size. No private reference movie, narration, source frames, or captures are shipped.

`homepage-revision.css` retains the accepted sequential layout and photo annotations. Photo markup lives in `PhotoStory.tsx`; social dialog markup
and interaction live in `src/footer/`. All are part of the normal source build.

## Greeting frog Easter egg

Two activations within 420ms trigger the greeting frog’s backflip, belly-flop, water ripples and QR reveal. The QR encodes `https://app.tryapparent.com/` and is a link for same-device use. Single activation retains the wave. Escape or Back to the pond restores focus and scrolling; reduced motion skips the flight and ripples. The mounted frog owns a React portal dialog with explicit reveal stages. Its effect cleans timers, animation frames and temporary page filters on dismissal, pagehide or unmount. Design tracked in #709.

## Brand review candidate

Public keeps the capture story, founder photos, frogs and source media. Page-owned
type uses Host Grotesk at 400/500/700; annotations use the same face and readable red
color. Marketing frames are square and flat. Playback actions use opaque paired
colors. Compact wordmarks are at least 120px wide.

Public maps the shared semantic roles to Paper/Carbon/Signal in light mode and
Carbon/Paper/Lilac in dark mode. Light-mode grouped sections use Stock with Carbon
text and Signal actions. Dark-mode grouped sections use a 12% Lilac/Carbon mix with
Paper text and Lilac actions. The demo, belief, founders and footer share that
muted field; original media keeps its colors.
This Public-only inverse mapping does not change Desktop or Connect tokens. Both
schemes still follow the system while open. No manual or persisted override is
introduced. The inverse mapping is a review proposal, not a claim of founder
approval. Publication remains on hold pending review of this candidate.

## Feedback review refinements

The demo heading uses the boxed wordmark. Illustrative feature frames inherit the
parent semantic dark palette, while document images and recorded media stay intact. Its segmented timeline has contrasting
8px tracks and named 44px chapter controls; phones show every chapter in two rows.
On phones, an enlarged 4:5 viewport follows the recorded cursor using the approved
movie's observed track. The media clock drives a smooth, clamped camera path;
seeking and replay use that same path. No per-frame React state or manual close-up
selector is used. Observed burned-caption text stays fixed over the subtitle band,
outside the moving video transform; cue bounds are sampled from the source. Desktop and
reduced-motion playback retain the full frame. Fullscreen always shows the complete
recording. The track records its source hash and observation limits beside its data.

The second and fourth feature rows reverse media/copy columns on desktop; phones
retain copy before media. The grouped field continues through the belief, team,
CTA and footer with no credit-bar divider. Red labels and the work circle use deep
red on light Stock and a lighter red on the muted dark field. The QR splash color wash is
90% opaque; the original QR image and dismissal control stay opaque.

# Demo motion loop

Tools for measuring the stage demo's motion. Serve a build first (`bun run build && bunx vite preview --port 4602`).

- `capture.ts` seeks the demo through `window.__apparentDemo.seek(ms)` at a fixed 60 fps of demo time, steps every CSS animation by the same dt, and writes `frames/*.png` plus `trace.json` (rects, drawn opacity, transform and filter of the tracked elements). `--slow 4` captures 4x as many frames for 0.25x clips; `--size iphone` uses 393x852 at 2x; `--query look=3` picks a look.
- `detect.ts <trace.json> --verbose` reports pop-in, pop-out, snap, reversal, layout-shift and size-snap findings, glowing text and the largest blur.
- `clips.sh <before> <after> <out>` builds a side-by-side h264 clip and a frame strip from two captures of the same range, and checks the strip for blank tiles.
- `hitches.ts` plays the demo at 1x and reports rAF interval percentiles and long tasks. Set `PW_WEBKIT` to a WebKit `pw_run.sh` if Playwright's pinned revision isn't installed.

Motion values live in `src/demo/motion.ts`. On the dev server, hover the stage for the knobs chip.
`?timescale=0.25` plays the demo in slow motion in any build.

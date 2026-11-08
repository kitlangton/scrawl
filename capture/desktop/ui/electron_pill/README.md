# Recording pill

Desktop's React/TypeScript row renders the existing Rust capture state through
bounded parent-owned pipes. Rust validates revisions, action availability,
capture generation and exact handoff selection before dispatch. The renderer
receives no bundle paths, credentials, configuration or capture contents.

The row covers selection, recording, processing and completion. It reuses
shared shadcn Button controls and Lucide icons. Cancel/dismiss is the rightmost
×; ordinary actions retain labels. Warnings and rejected actions remain inline;
there is no expandable panel, notes extension or Record-package dependency.

Build the frontend with `npm ci && npm run build` in `../frontend`. Its build
transpiles the preserved CommonJS Electron host from `.cts`; generated `.cjs`
files are ignored and included by the existing Settings packaging route.
`npm run test:pill` exercises the real Electron main/preload/renderer with a
synthetic parent; it is not native capture acceptance.

The existing Desktop coordinator owns child startup and readiness. A failed
launch, expired readiness or child exit restores the native pill without
stopping capture. Linux warms the hidden child at startup and retains native
held-button/identity checks, drag clamping and exact-window pinning. The child
alone moves/remaps its Electron surface; Rust never remaps it concurrently.
The child also reconciles its X11 input stacking without taking focus.
macOS retains native caption
dragging; Glass or explicitly disabled movable controls retain native routing.
There is one visible pill. Missing packaged assets retain the native route.

Rollback is removal of the Electron routing calls and fields in Desktop's
coordinator; the native implementation remains required for failure recovery
and the compatibility routes above. Remove it only after accepted replacements
cover those consumers and both platforms, including startup/crash recovery,
focus, stacking, dragging, capture visibility and actual Paste delivery.

Native acceptance must use the exact packaged candidate in isolated Linux and
macOS environments. Synthetic renderer checks do not establish that acceptance.

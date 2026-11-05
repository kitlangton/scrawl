# Shared UI sources

Canonical React/shadcn controls and system appearance for every value stream.
Named icon exports from `@apparent/ui/icons` use pinned `lucide-react` so
consumers share one icon source. Live microphone levels remain data-driven bars.
Desktop, Connect account and recording recipient pages, and Public consume the
same framework-independent tokens. Engine and Agent frontends must use them too. Application policy,
account state and capture state do not belong here.

Button, Tabs, Collapsible, Alert and `cn` come from shadcn/ui's MIT-licensed `new-york` registry,
retrieved from `https://ui.shadcn.com/r/styles/new-york/{button,tabs,collapsible,alert}.json`.
Only import paths and formatting were adapted. Review source changes on updates;
update underlying Radix and utility packages normally. Add controls here, not
separate Desktop/Connect copies. Theme tokens are shared, not OS-native skins.

The Desktop foundation uses a `file:` dependency with npm `install-links=true`,
so npm installs a snapshot rather than resolving dependencies from this source
directory. After changing this package, run `npm ci` in
`capture/desktop/ui/frontend` to refresh that snapshot. Its build checks that the
installed source matches these canonical files.

StatusCard and DisclosureMenu supply the shared Connect surface. `status.css` owns
functional status roles from the Apparent brand guidelines: green confirmation,
Signal progress, gold/Carbon caution and red failure/destructive actions. Ordinary
Tab navigation, Escape and outside-click dismissal are retained by the disclosure.
Rust owns state, authorization, action placement and confirmation decisions.

## System appearance contract

Import `@apparent/ui/tokens.css` for plain CSS, or `@apparent/ui/theme.css`
with Tailwind v4. Repository Vite consumers without a package dependency import
the canonical tokens.css source directly; this is a shared implementation, not a
copy. Tokens apply on `:root`, including portals and native form controls, and
follow `prefers-color-scheme` at startup and while open. Do not persist a theme
override. Use semantic `--color-*` variables or Tailwind classes such as
`bg-surface text-foreground`; never add a consumer-owned palette or forced scheme.
Pair primary/destructive/secondary backgrounds with their matching foreground.
Keep captured media and illustrations unchanged. Brand-colored patches need a
paired brand foreground; translucent fades use the current background token.

Electron uses `@apparent/ui/window-theme` to keep its native window background
in sync with the same CSS tokens; it removes its subscription when closed.
The installed routing/rollback stays unchanged: native surfaces remain native,
and existing Electron routing selects its existing surfaces. No legacy native
renderer is removed by theme adoption.

Every frontend build and repository PR verification runs
`node shared/ui/scripts/check-theme.ts` (the legacy Python entrypoint forwards to the same implementation). The per-file legacy color inventory
reserves existing demo/artwork and unmigrated literals; new files have no
allowance. Reduce it when migrating an old surface. New exceptions require a
reviewed explanation; it is a ratchet, not proof of visual correctness.

After building Desktop, Connect web/service and Public, run
`node --test shared/ui/test/contracts.cjs` and
`node shared/ui/test/appearance.mjs` from the repository root. The latter checks
actual packaged surfaces in Chromium and Firefox through live light/dark/light
switches and writes synthetic screenshots to `THEME_EVIDENCE` (or a new temp
directory). `PUBLIC_THEME_ARTIFACT=public/site/dist-apparent` tests the final
Pages snapshot plus its deterministic color-only appearance adapter. Web delivery
runs this check before publishing; Desktop CI runs the Electron appearance smoke
and shared contract tests. `THEME_SURFACES` and `THEME_BROWSERS` select available
consumers/engines for those jobs. This is browser evidence; macOS and isolated Linux native theme
propagation remain separate platform acceptance gates.

For native OS propagation, stage Settings using
`capture/desktop/scripts/packaging/stage-settings.py`, then run the staged Electron
runtime with `shared/ui/test/native.cjs` in an owned disposable macOS/Linux guest.
Set `APPARENT_THEME_APP` to its staged `app` and `SCRAWL_GUI_OUTPUT` to a private
evidence directory. The test changes only the guest's OS appearance, restores it,
and checks CSS, native window background and app-owned screenshots. It requires
`gsettings` color-scheme support on Linux or System Events appearance on macOS;
a missing system appearance route is an unrun gate, not a passing emulation.

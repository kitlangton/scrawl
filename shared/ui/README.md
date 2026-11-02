# Shared UI sources

Canonical React/shadcn controls and Tailwind v4 light-theme tokens. Desktop is
its first consumer; Connect is not migrated by this package. Application policy,
account state and capture state do not belong here.

Button, Tabs and `cn` come from shadcn/ui's MIT-licensed `new-york` registry,
retrieved from `https://ui.shadcn.com/r/styles/new-york/{button,tabs}.json`.
Only import paths and formatting were adapted. Review source changes on updates;
update underlying Radix and utility packages normally. Add controls here, not
separate Desktop/Connect copies. Theme tokens are shared, not OS-native skins.

The Desktop foundation uses a `file:` dependency with npm `install-links=true`,
so npm installs a snapshot rather than resolving dependencies from this source
directory. After changing this package, run `npm ci` in
`capture/desktop/frontend` to refresh that snapshot. Its build checks that the
installed source matches these canonical files.

StatusCard and DisclosureMenu supply the shared Connect surface. `status.css` owns
functional status roles from the Apparent brand guidelines: green confirmation,
Signal progress, gold/Carbon caution and red failure/destructive actions. Ordinary
Tab navigation, Escape and outside-click dismissal are retained by the disclosure.
Rust owns state, authorization, action placement and confirmation decisions.

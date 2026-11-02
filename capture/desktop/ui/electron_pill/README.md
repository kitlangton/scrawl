# Shared recording pill

React/TypeScript and shared shadcn controls render Rust's bounded `View`.
Rust retains selection, capture admission, microphone warnings, processing,
clipboard/paste, sharing policy and finalizer receipts. The child receives no
bundle paths, capture contents, config or credentials. It cannot choose a
capture ID, path, URL, shell command or window coordinates.

The parent launches the packaged `electron/pill/main.cjs` from the existing
Desktop Electron runtime. Development builds resolve the built frontend beside
the source checkout. Linux selects X11 on the process command line, before
Electron initializes Ozone; a switch appended in JavaScript is too late on
Wayland desktops. There is no second backend or renderer recording clock.
State frames carry an action revision; Rust checks the current action set and
capture generation, consumes admitted revisions, and refuses expired commands.
Pipes and queues are bounded; writes and process reaping run off the GUI thread.
On Hyprland, show/move/resize notifications coalesce into bounded background
pin attempts through the existing Linux adapter. It matches the owned Electron
PID and XID-specific title; other Apparent windows keep their workspace behavior.

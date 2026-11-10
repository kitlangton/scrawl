# Native layout dependency patch

`taffy/` is the published crates.io Taffy 0.9.0 package (MIT), with its
upstream license restored from the `v0.9.0` tag. Source, examples, README and
original manifest are unchanged. The normalized `Cargo.toml` changes only the
`grid` requirement from `0.18.0` to `1.0.1`.

GPUI 0.2.2 pins Taffy exactly to 0.9.0. The root Cargo patch preserves that API
and layout implementation while removing grid versions affected by
[GHSA-38c5-483c-4qqp](https://github.com/advisories/GHSA-38c5-483c-4qqp).
Upstream Taffy made the same dependency-line upgrade in
[`79e45897`](https://github.com/DioxusLabs/taffy/commit/79e45897f69abb28ef5c81452540a13f3852dc1d);
we require at least 1.0.1 because 1.0.0 is affected too.

This is third-party native layout execution, not Record workflow policy or
Desktop presentation. It adds no configuration writer or routing change.
Retain upstream's directory layout for review and updates; this dependency
tree is excluded by the repository's directory-width guard.

Verify the embedded upstream unit tests in a disposable copy of `taffy/`, seeded
with the root lockfile, using `cargo test --manifest-path <copy>/Cargo.toml --lib
--offline` (Cargo cannot test a non-workspace dependency with `-p taffy`). Check
that its resolved dependency versions match the root graph. Also run root
build/tests and supported macOS/Linux native layout checks. A successful build
is not GUI acceptance. Inspect `cargo tree -i grid --locked` for the root graph.
Remove this directory and the root patch together when the selected GPUI
release accepts a published Taffy whose graph resolves fixed grid versions.
Do not replace this patch with a duplicate direct grid dependency or an alert
suppression. Tracking: [#1159](https://github.com/tryapparent/apparent/issues/1159).

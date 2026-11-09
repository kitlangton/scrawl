Ready-PR/main platform verification opts into root test/build overlap. The
verifier retains the Cargo inventory; `root.ts` starts the real app build only
after Cargo reports successful test compilation. Two test threads overlap one
build worker. Both commands must pass before the verifier continues. Focused
filters, allocations below three workers and ordinary local callers retain the
sequential path. Engine, native, cache and publication gates remain required.

Each overlap has an owned session leader. It stays alive after command exit and
signals only its own process group on completion, failure, cancellation or IPC
disconnect. The caller waits for that leader to exit; it never signals a possibly
reused child PGID. The enclosing workflow still owns its existing job deadline.

`inputs.ts` supplies the committed, no-rename path facts to hosted and local
selectors. Hosted execution/acceptance stays conservative. Local feature pushes
may classify modifications to existing regular `checks.yml` and
`stream-public.yml` files as control-only: the ordinary top-level concurrency
block must change, parsed execution fields must match, and bytes outside that
block and adjacent comments must match. YAML anchors/tags/aliases or complex
control shapes, invalid/unavailable history or parsing, file additions/deletions,
renames and mode/type changes cannot establish that proof. Other changed paths
still select their own coverage; unknown/execution-sensitive inputs stay broad.

Eligible pushes run the mandatory theme, hook, selector, YAML/workflow,
prerequisite, timing and release-admission contracts plus tooling width checks.
Only unrelated product suites are omitted. Main/tag pushes disable this local
exception; explicit conservative verification, hosted platform gates and release
or native acceptance are unchanged. Selection tests prove command omission and
failure propagation, not measured per-push savings or native behavior.

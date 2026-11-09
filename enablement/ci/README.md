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

Mac's early CI-sensitive fixtures share one `cargo test` invocation. libtest
ORs the three positional substring filters, retaining both command-line row
cases, competing-launch socket ownership and Darwin ACL fail-closed checks.
They use two test threads, as the later full suite already does. The cases use
independent fixtures; the early groups no longer require separate processes.
Linux's early commands are unchanged. Packaging, native assertions, the later
unfiltered verifier and cache-completion/publication guards remain required.
No artifact or test receipt is restored by this change; cold caches still build.

Diagnosis used completed Intel Mac jobs on Rust 1.99.1, the unoptimized root
test/dev profiles and the same full three-workspace cache inventory:

- [137d9605e main job](https://github.com/tryapparent/apparent/actions/runs/42609778162/job/129135889251):
  exact 748 MiB dependency-cache restore took 136s. The early Cargo commands
  reported 325s, 94s and 71s; only the first reported compilation (root and
  path client). The four harness cases reported 0.33s combined execution.
- [15132a092 main job](https://github.com/tryapparent/apparent/actions/runs/42614044719/job/129153813172):
  the same exact cache restored in about 73s. Early Cargo commands reported
  403s, 71s and 75s. Later retained compiler HTML had 721 fresh units, zero
  dirty units and no compiler activity while its command span took 61.7s.

Those observations identify repeated Cargo preparation/freshness work, not
repeated linking in the later two calls. They do not localize the fresh-check
delay further or separate the first build's native helpers, Rust codegen and
linking. Early `--timings` now retains Cargo build-unit timings through the
existing artifact upload; it does not time individual Swift or linker calls.
Removing two invocations targets their observed
overhead; it is not a measured end-to-end speedup. Workspace-crate caching is
unchanged: source/config, compiler/profile/features and native build inputs
remain Cargo's responsibility, without copying targets across worktrees.

The ordinary prerequisite contracts exercise workflow ordering and failure
gates without Rust. For cold-build, real Cargo/libtest selection and a failing
assertion in each of the four cases (synthetic dependency-free sources):

```sh
APPARENT_EARLY_CARGO_FIXTURE=1 node --experimental-strip-types --test enablement/prerequisites/order.test.ts
```

On Clanker, run that command through `t3-build --jobs 3 --min-jobs 3 --`.
This fixture proves filter semantics and failure propagation, not Mac native
acceptance. Normal ready-PR platform CI must still verify the real candidate.

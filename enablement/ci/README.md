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

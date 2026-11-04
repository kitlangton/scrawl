# Standalone capture control

This is the existing Rust GUI capture-control client extracted for independent
build and installation. The full CLI and Desktop provider adopt this same source.
It does not launch a provider, run a daemon, or implement media capture.
Supported platforms: Linux and macOS.

Build/test without the repository root package or native media dependencies:

    cargo build --locked --offline --manifest-path capture/agent/control-client/Cargo.toml
    cargo test --locked --offline --manifest-path capture/agent/control-client/Cargo.toml
    cargo package --locked --offline --manifest-path capture/agent/control-client/Cargo.toml

Install in a dedicated prefix (choose an absolute private path), then invoke
PREFIX/bin/apparent-capture. Do not use a prefix containing a previous installation
unless retaining its binary first:

    cargo install --locked --offline --path capture/agent/control-client --root PREFIX

Rollback: remove only PREFIX/bin/apparent-capture, or restore the saved previous
binary. No config migration, Desktop installation change, or replacement of
'apparent' occurs. Cargo's install metadata remains within PREFIX.

Commands are state, start [--request-id ID] [--wait] [--timeout SECONDS],
inspect ID, stop ID [--wait] [--timeout SECONDS], cancel ID. IDs are 32 hex
characters; generated start IDs come from /dev/urandom. JSON receipts and exit
failures match 'apparent capture'. Only start and stop support waiting; timeout
is bounded to 1–3600 seconds (defaults 30 and 60).

## Installed-provider compatibility and safety

The installed GUI must already support its existing capture.sock text protocol:
newline-delimited 'state' or 'ACTION ID MONOTONIC_DEADLINE_MS', JSON reply.
There is no new version handshake or provider upgrade. Older providers lacking
this endpoint fail as control_unavailable. Typed drawing/reference/share actions
remain provider/full-CLI functionality and are not exposed by this executable.
This extraction preserves the existing protocol rather than introducing v2.

The endpoint is HOME/.config/apparent/agent-control/capture.sock. The directory
must belong to the effective user and have no group/other permissions. The
socket must be an owned, mode-0600 Unix socket; peer credentials must match the
effective user on Linux/macOS. This is same-user authorization, not isolation
from other processes running as that user.

Each exchange has an absolute deadline capped at four seconds, including connect,
write and read; replies are bounded to 4096 bytes. Waiting sends the mutation
once then inspects only the original ID. A timeout never cancels an admitted
capture or retries a stop toggle. Inspect the same ID before retrying after a
transport failure. Receipts live only for the provider lifetime: after restart an
unknown ID is not authority to control a new capture. Global state never grants
ownership. Stop/cancel remain subject to the installed GUI's matching-ID admission,
consent and safety checks. No config or private recording data is rewritten.

The transport/semantic tests use isolated synthetic sockets and receipts.
They are not native capture proof. Before merge, exercise this binary against an
installed supported GUI in an approved isolated Linux/Mac environment, including
owned start/inspect/stop/cancel and restart/timeout recovery, and inspect retained
bundle text/images. No real host screen or new rental is authorized by this work.

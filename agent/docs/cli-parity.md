# Agent CLI parity

This inventory is based on the active parser and handlers in `composition/cli.rs` at the Agent CLI parity campaign's starting main (`73411840`). The old `apparent` command remains authoritative for commands not listed as implemented below. Agent work is owner-local TypeScript under `agent/`; it does not change installed routing.

## Command inventory and ownership

| Legacy command family | Current owner and behavior | Agent parity status / next dependency |
|---|---|---|
| `inspect` | Agent + Engine bundle contract; local manifest, semantic summary, marks, JSON and experiment options. Rust also resolves Connect IDs. | **Partial: local inspect output.** Agent owns validated local JSON, marks and generated summaries; semantic parity is tracked by current, legacy and producer-shaped Rust goldens. Local bundles only; no URL/Connect lookup. |
| `record` | Engine capture/media and Desktop/native adapters; timed capture, iPhone, optional note/output. | **Partial:** Agent foreground adapter accepts an installed Engine protocol provider, but the merged Engine native provider has no recorder/media API. Native capture parity awaits Engine capability publication and platform proof. |
| `capture state`, `capture start`, `capture stop`, `capture cancel`, `capture inspect` | Agent owns its private request journal; Engine owns request receipts/state; Desktop owns the running GUI lifecycle. | **Partial:** Agent `capture inspect|stop|cancel` only covers requests journaled by this Agent. GUI state/start/stop/cancel remain Desktop-owned. Engine native provider and real operation proof remain prerequisites. |
| `frames` | Engine media/frame extraction. | Not implemented. Next dependency: a published Engine frame extraction capability with local-only artifact semantics. |
| `zoom` | Engine media and semantic context; emits a dense contact sheet around time, phrase or mark. | Not implemented. Depends on local frame extraction and mark/time lookup semantics. |
| `export` | Engine media pipeline and platform encoders; H.264/HEVC, crop, zoom, timing, ink, clicks and audio policies. | Not implemented. Depends on published Engine export/media capabilities and affected-platform proof. |
| `reprocess`, `retranscribe`, `recover` | Engine pipeline, transcription and bundle durability. | Not implemented. Depends on Engine processing and safe publication APIs. |
| `clean`, `pin` | Engine bundle retention/durability. | Not implemented. Depends on an Engine-owned local storage/retention contract. |
| `copy` | Desktop clipboard and handoff UI/native integration. | Not implemented. Depends on an Agent-safe handoff contract; no session posting is implied. |
| `preview` | Desktop GPUI review window. | Not implemented. UI-owned; preserve the installed desktop behavior. Agent CLI is not a review workspace. |
| `app`, `launch`, `launch-handler install|status`, `shortcut record|paste`, `devices`, `mic list|set|watch` | Desktop lifecycle, OS registration, shortcuts and native device enumeration. | Not implemented. Desktop/native owner routed; requires published OS integration capabilities and per-platform checks. |
| `update status|channel|check` | Desktop Linux AppImage updater. | Not implemented. Desktop/Linux updater owner routed. |
| `hex status` | Engine transcription provider readiness. | Not implemented. Provider-owned; requires a published status capability. |
| `connect` (`cloud` alias): `login`, `status`, `logout`, `disable`, `retry`, `upload`, `shares list`, `shares revoke`; `fetch` | Connect authentication, upload queue, hosted retrieval and consent. | **Deferred by user.** No Agent network, auth or hosted-fetch work in this campaign slice. |
| `issue` | Composition/GitHub feedback submission. | Not implemented. Keep separate from capture inspection and require its existing explicit submission policy. |
| `demo` | Desktop/Engine synthetic bundle generation and optional clipboard. | Not implemented. Depends on a supported synthetic bundle producer; no capture implementation in this slice. |
| `skill` | Agent skill content. | Existing Rust CLI prints the packaged skill. The Agent package does not yet provide this command. |
| `autocrop` | Engine crop policy plus Desktop/browser playground. | Not implemented. Engine owns crop semantics; interactive playground remains UI/provider routed. |
| `lab pill`; `tune list|set|preset|reset|rust` | Desktop developer motion lab and tune settings. | Not implemented. Developer/UI owned; keep debug configuration and surfaces with Desktop. |
| `fixture`, `capture-sim`, `autocrop-eval`, `windows`, `sck-probe`, `decoder-check`, `encoder-check` | Engine/Desktop developer diagnostics, simulation and platform probes. | Not implemented. Developer/provider owned; each requires its relevant native or corpus capability. |

The table includes hidden, Linux-only, Mac/Linux-only, developer and provider-owned parser entries. Platform gates and hidden status are properties of the legacy parser, not claims of Agent support. Connect is a selected command-family inventory, not an exhaustive list of its subcommands; it includes the legacy `cloud` alias. Options and platform-specific constraints remain authoritative in the installed legacy `apparent --help`.

## Local inspect contract

`apparent-agent inspect [BUNDLE|latest]` resolves only local bundle paths. Default text is regenerated from the validated `apparent.json` manifest, so absent or stale `summary.md` cannot hide current manifest semantics. `--stored-summary` explicitly prints bounded producer-authored text. `--json` writes the original manifest bytes unchanged; `--marks` and `--marks --json` retain the established Rust mark shape and ordering. `--transcript` and `--images` remain local conveniences. `--experiment agentContext` selects the richer event/mark timeline for this inspection only; a `true` value in `experiments.agentContext` in the existing config also enables it, and an explicit flag overrides a configured `false`. The default remains off and inspection never writes config. Unknown experiment names are accepted as Rust's repeatable option accepts them and have no Agent formatting effect. `--stored-summary` cannot apply this formatter and conflicts with that experiment option.

Inspection never fetches a URL, invokes Engine/native code, decodes media or changes bundle/config files. Manifest and explicit stored-summary reads are strict UTF-8, no-follow, regular-file reads capped at 64 MiB. Image listing validates safe relative references without opening image contents. The generated text retains local absolute bundle path, app/device/window context, app context values, notes, transcript, marks/events, available audio/transcript warnings, crop and image-quality warnings, and instructions to inspect visual files. agentContext uses timed words, clause snapping, keyframe ownership, settled interactions, window changes and context changes. WAV presence requires a bounded local no-follow read of a valid WAV header and first PCM8 or PCM16 sample; no decoding, encoding or native media execution occurs. Text is semantic; pictures remain bundle files.

## Acceptance and retirement boundary

This slice is accepted when the Agent owner suite, packed consumer install, and Rust-free TypeScript build all pass; current, legacy and producer-shaped synthetic fixtures match Rust output after substituting only the temporary bundle path; inspection leaves manifest/config bytes unchanged; and the built package works outside the checkout without Desktop, Rust or network access. This semantic-only change has no native OS boundary. No new `apparent` routing is selected here.

The legacy Rust command remains until every supported installed/package-manager consumer has moved and each command family's owner has accepted its replacement. Agent package publication, app command routing and donor deletion are separate campaign gates; a parity entry in this table is not evidence that those gates passed.

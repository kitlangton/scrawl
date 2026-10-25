# Smart routing

Read this before changing `src/routing/smart.rs` or `src/routing/look.rs`. The ladder that runs when the experiment is off is described in `docs/iphone-and-routing.md`, and the words used here are defined in `CONTEXT.md`.

Smart routing is an experiment, on with `"experiments": {"smartRouting": true}` in `~/.config/scrawl/config.json` (read through `handoff::experiment`). With it off the app calls `routing::route_capture` exactly as before. With it on, a Mac Screen or Window capture is routed in three steps.

1. `look::look_live` runs while the bundle is built. It reads the browser tab in front (or captured) and turns it into clues: the page's `/sdk.js` registration, and the folder of the process serving its port when the tab is on this Mac (or, when that process serves the page from outside its own project, the page's file and its publisher: see below). "On this Mac" means a loopback host or one of this Mac's own names (`hostname`, its `.local` name, its Tailscale name), whose port is mapped through `tailscale serve status --json`. It reads Ghostty's focused pane (folder and title) when Ghostty is captured or in front. Then it reads the open root sessions: the newest `SESSION_PAGES` pages, the newest page in each folder a clue names, and each session a clue names.
2. Once the bundle exists, `look::bundle_clues` adds clues from it: the window title minus the app's own name, the captured app's name unless it's a browser, terminal, editor or system app, the checkout the captured app is built from, the transcript, and for OpenCode Desktop the session viewed last. The recorded destination becomes the address. `smart::decide(&evidence, Policy::Weigh)` returns a judgement. `routing::judge_capture` saves the evidence and decision as `routing.json` in the bundle and logs one line.
3. `ScrawlCoordinator::act_on_route` acts on it when it arrives, after the capture has been copied and shown as Completed (routing never delays that; ✕ or Paste before it arrives keeps the capture on the clipboard). Session reads are bounded as described in `docs/iphone-and-routing.md`.
   - A send shows "Sending to DESKTOP…" for `SMART_SEND_UNDO` (3 s), with the runner-up as a "Send to OPENCODE IOS" chip beside Paste. ✕ or Paste cancels the send; the chip sends there instead, at once. Then the destination is written to `scrawl.json` and `summary.md` and sent through `send_completed` (and so `delivery::deliver`, with its retries).
   - An offer asks: the Completed island keeps "Copied" and adds "Send to DESKTOP" and the runner-up ("OPENCODE IOS") beside Paste, which stays the default. Nothing is sent until a chip is clicked; a click is the user's choice (`via: "chosen"`). The review window's status names the suggestions too.

## Site rules

Some sites always belong to one project, whatever their tab title says (Problems titles its tabs "<project> · Problems"). `"routing": {"sites": {"<URL prefix>": "<folder or session ID>"}}` in config.json says so: `{"https://problems.kitlangton.com": "~/code/worktrees/team-board"}`. A prefix covers its host and any path below it, query and fragment aside; without a scheme it covers `http` and `https`, and without a port any port. The longest prefix wins. `routing::site_rule` matches it, and `look::url_clues` turns it into a `Rule` clue: a session ID as it is, a folder as the folder (its repository when it's in one) with every folder below counting.

`smart::decide` obeys the rule after an address and before weighing: the session it names, or the newest open session in the folder (else anywhere in its repository, worktrees included) is sent to with no undo-window runner-up, recorded `via: "rule"`. A folder no open session works in leaves the capture to weighing. With smart routing off, `routing::route_with_sites` sends to the newest open session under the folder (or its repository's root) ahead of the ladder; it doesn't look in sibling worktrees.

## Pages from a file server

A server can serve files that aren't its project's: GPU Gallery (`gpu-gallery/server/serve.ts`, behind `:8444`) serves `/raw/<path>` and `/<path>` from `~/code/home/artifacts`, where any session publishes. Its working directory says nothing about who made such a page, so `look::look_at_page` (after the sessions are read) finds the page's file with `served::locate`:

- `"routing": {"servers": {"<URL prefix>": "<folder>"}}` in config.json maps a prefix outright, longest first (`"https://studio.example.ts.net:8444/raw/": "~/code/home/artifacts"`).
- Otherwise, when the URL's path names a file (an extension, or a trailing `/` read as `index.html`), the longest tail of it (at least the file and its folder) is looked for in the server's project and its immediate subfolders, then in the folders and repositories open sessions work in and their immediate subfolders. Found in the project, the port clue stands. Found once elsewhere, the page is that file. Found nowhere, or twice at the same length, nothing changes.

A server can also run inside a folder sessions publish into, such as the onboarding story's server in `~/code/home/artifacts/2026-10-20-onboarding/story`. Its folder's repository (`~/code/home`) and that repository's newest session (HOME) say nothing about who made it. `served::content_root` finds the publishing folder: a `routing.servers` folder or a folder named `artifacts` above the server's folder, else the folder above the nearest one holding an `.opencode-session`, all within the server's project. If no open session works in the server's folder itself, the port clue is worth `served::SERVER_IN_CONTENT` (0), and the publisher is found as below from when the folder's newest entries changed (`served::changes`: up to 5, within 2 hours of the newest, `node_modules` and hidden ones aside). The session that wrote the most of them gets 95 (each, on a tie); one that wrote fewer gets 60.

For a page from elsewhere the port clue is worth `served::SERVER_ONLY` (15), so the server's session is at most a runner-up, and the file gives the clues:

- **Publisher, named** (100): `<meta name="opencode-session" content="ses_…">` in an HTML page, else an `.opencode-session` file (the session ID) in its folder or one above it, below the served folder. A subagent's session is followed to its root through the API. Agents publishing to the gallery write one (`~/code/home/AGENTS.md`; `$OPENCODE_SESSION_ID` holds the ID).
- **Publisher, from history** (95 for a session whose tool call was running when the file last changed, 60 for one whose calls named its folder in the 30 minutes before): `served::opencode_writes` reads the local OpenCode server's database read-only (`/usr/bin/sqlite3 -readonly`, found through the pid `/api/info` reports and `lsof`; 3 s limit; nothing when `agentHost` is set) for tool calls whose input names the page's folder below the served folder (`2026-10-19-hex-shout2`), in messages started within that window, with each call's root session. OpenCode's API has no search across sessions; the query depends on `session_message` and `session_v2`, and any failure is no clue.
- **Document** (40): the words of the file's path below the served folder.

`scrawl route --explain` does this for older captures too, with the URL their destination recorded and the file and history as they are now.

## The captured app's checkout

When the captured app is software whose source Kit works on, that checkout is a clue (`Source::App` with a folder, 70 points, like a terminal's folder). `look::app_checkout` finds it:

- `"routing": {"apps": {"<bundle ID or prefix>": "<folder>"}}` in config.json, for apps whose checkout is named otherwise (`"com.kitlangton.problems": "~/code/worktrees/team-board/rewrite"`). The longest matching prefix wins. With Jev routing on, such an entry is a rule instead (see "Who sends on their own"). General-purpose apps (`policy::GENERAL_APPS`: browsers, terminals, OpenCode, Finder, Slack, Messages, Mail) are dropped from `apps` when config is read, with a log line.
- Otherwise the one open session repository (or folder, outside a repository) whose name is the app's name run together or one of its words or bundle ID parts, generic ones dropped: OpenCode Beta (`ai.opencode.desktop.beta`) → `…/opencode`, Scrawl Helper (`dev.kitlangton.scrawl.helper`) → `…/scrawl`, Hex → `…/hex`. Two checkouts named alike give nothing. Browsers, terminals, editors and system apps give nothing, OpenCode aside.

There are no built-in paths: the defaults for OpenCode and Scrawl come from their checkouts being named `opencode` and `scrawl`, so they work on any Mac where a session is open in them.

## OpenCode Desktop: the session or the app?

Most captures of OpenCode Desktop are about OpenCode itself (the composer, a picker, the sidebar), not about the session it shows. `look::about` reads what was said, the Accessibility roles of what was clicked and what marks were drawn on:

- **Interface**: two or more interface words ("button", "composer", "picker", "spacing", "search"…) or clicks on controls, and no talk about the session's work. The app's checkout gets 70 and the viewed session 15.
- **Session**: phrases about the agent's work ("you said", "your plan", "this diff"…) at least as many as interface words. The viewed session gets 60 and the checkout 30.
- **Unclear**: anything else. The checkout gets 60 and the viewed session 30, so the viewed session never beats the app's own repo when it can't be told, and neither sends on its own.

## Scoring

A candidate's score from clues is the sum, over sources, of the best points any one clue of that source gives it. Recency is added to that if it's above zero. The points are in `Source::points` (or a clue's own `points`, as above) and the thresholds are `SEND_AT` (95), `LEAD` (15), `OFFER_AT` (40) and `ALT_AT` (15).

- Page (`data-session`: 100, `data-directory`: 95), Port (95, or 15 for a page from elsewhere) and Publisher (named: 100, wrote the file: 95) send on their own. Terminal (70), the app's checkout (70), Viewed (60), Publisher from calls that only named the folder (60), App name (50), Title (40), Document (40) and Speech (25) don't.
- A folder clue gives full points to the sessions closest to its folder (on the line from the project root to it, or inside it), 15 fewer per extra folder for sessions further along that line, and half to a session elsewhere in the project or in another checkout of the same repository (`look::repo_of` follows a worktree's `commondir`).
- A word clue's points are divided by the number of candidates the word names. A word names a session through its whole title or its folder's or repository's whole name (always), a word of a longer title, or a piece of its folder's name. Ordinary words (`smart::COMMON`: "loading", "search", "button", "chat"…) only count as a whole name, because folder and title words say what the work is about and the same words are said about everything. A piece of a worktree's name counts half, and OpenCode's made-up worktree names (`…/worktree/<hash>/lucky-rocket`) name nothing.
- Recency: 20 within 10 minutes of the capture's end, 12 within an hour, 5 within a day, counting the session's update or view times up to a minute after the end.

## Deciding

Candidates rank by their clue score; recency only breaks ties.

- **Send** when the best has at least `SEND_AT` from clues alone and leads every other candidate by `LEAD`, recency included. So recency can separate two sessions a port names equally, but never lifts a session to `SEND_AT`.
- **Offer** (ask) when the best scores at least `OFFER_AT` otherwise.
- **Keep** below that, with the reason.

A send and an offer both carry runners-up (`smart::alternatives`, at most two): a candidate within `LEAD` of the best, or one resting on a different source than the best with at least `ALT_AT` from clues, one per source. That's the session a misread clue would have meant: the viewed session when the app's checkout leads, or the reverse.

## Jev

With `"experiments": {"jevRouting": true}` as well, `routing::jev` asks Jev (`jev-1.13`, TypeSafe's System One model on OpenCode's gateway) instead of weighing words. Deterministic sources still win: when smart routing would send on an address, a session named at the start (below), a choice, a recorded destination, a site or app rule, a page's `/sdk.js` session, a publisher, a localhost port or a dev build (`jev::settled`), Jev isn't asked. What happens to Jev's own sends is `routing.autoSend`'s call (below). Otherwise the title, speech, app, viewed-session and terminal clues go to Jev as sentences in `hints`, beside the capture (transcript trimmed to 2000 characters keeping both ends, mark and click notes, app, window title, URL, mode, duration) and up to 20 targets `t1…tN`, newest first: the sessions a clue names, smart routing's top three, HOME, then the most recent, each with its title, folder and when it was last active. `"jevRouting": {"lines": true}` adds each one's last user and assistant lines (read through `/api/session/{id}/message` in parallel within 1.5 s; replays read OpenCode's database as it was at capture time); they're off by default because the first eval picked right more often, and cheaper, without them.

Beside them goes `screen`, text only (`jev::Screen`): `windows` (`{app, title}`, front to back, at most 12), `focused` (`{role, title, value_excerpt}`, 500 characters around the cursor), `visible_text` (1500 characters of the front window), `opencode_tab` (the OpenCode tab in front) and `recent` (the last 3 dictations, `{text_excerpt, app, routed_to}`, 200 characters each). Hex sends these with dictation; a capture fills `windows` from its last window snapshot (regular apps' normal windows) and `opencode_tab` from the session a ScrawlKit timeline says the app showed, and no OCR. Every field is capped again here whatever the caller capped, and a field of the wrong shape is skipped. The target question then says to use `screen` (and each target's recent lines, when they're on). An `opencode_tab` that is exactly a target's title adds a hint naming it, which Jev still weighs. The request stays under about 8k tokens (`STATE_BYTES`, 30,000 bytes): `visible_text` shrinks first, then windows from the back, then the focused element, the earlier dictations and the tab; the targets never do. `screen` reaches only Jev and the saved request in `router/decisions/`, never `scrawl.log`.

Two questions: `target`, a choice between the targets, and `any_fits`, whether any of them fits at all. The decision (`jev::Band`, in the same words in `routing.json`'s `jev.decision`, the log and `scrawl router decide`), with thresholds overridden by `"jevRouting": {"send", "margin", "offer", "noFit", "low", "lines"}`:

- **send** with the usual undo countdown when confidence ≥ 0.85 and its margin over the runner-up's probability ≥ 0.3, whatever `any_fits` says. A runner-up above 0.1 rides along as the "instead" chip. Jev's target choice is well calibrated (52/53 and 22/24 right on HOME's voice eval) but its yes/no answers aren't (real positives landed between 0.3 and 0.65), so `any_fits` never blocks a confident choice. Captures and dictation share the rule.
- **ask** otherwise when confidence ≥ 0.5: up to four chips (`jev::CHIPS`), Jev's top two, then smart routing's best that isn't one of them, then HOME. `routing.json`'s `jev.chips` records each chip's session and where it came from (`jev`, `smart`, `home`), and `scrawl router decide` gives the same as each candidate's `from`. The GPUI capsule shows all four; the Metal capsule has two route slots and shows the first two. Here `any_fits` under 0.2 picks fallback instead.
- **fallback** below that: sent to HOME with the countdown (`via: "jev-home"`) and Jev's guess one click away. `"low": "keep"` makes it **keep** (clipboard only) instead, as does having no HOME session.

### Who sends on their own

Jev's pick, however confident, is a guess. `"routing": {"autoSend": …}` (`policy::AutoSend`) says what may send without asking:

- **`"rules"`** (the default): only deterministic sources (`policy::obeyed`): an address or a choice, a session named at the start, a `routing.sites` or `routing.apps` rule, a signed page or its publisher, the localhost port, and a dev build (below). Jev's send and fallback and smart routing's weighed send become an ask (`policy::suggest`) with the pick first, pre-selected, then its runners-up and the rest of the ranking, up to `jev::CHIPS` with HOME last. `routing.json`'s `jev` gets `"decision": "ask"`, `"suggested": true` and `"would": "send"` (or `"fallback"`), with the confidence kept, and the decisions log says the same.
- **`"confident"`**: the bands above as they are, Jev's send and fallback after the countdown.
- **`"never"`**: everything asks but an address or a choice; rules only put their session first.

`scrawl router decide` follows the same setting: a suggestion answers `"decision": "ask"` with `"suggested": true`, `"would"` and the pick as `target` and first candidate.

**App rules** (`Source::AppRule`, `via: "app-rule"`): with Jev routing on, a `routing.apps` entry covering the captured app (or Hex's `app.id`) is obeyed like a site rule, after it: a `ses_` ID as is, a folder's newest open session, else its repository's. `policy::app_rule` refuses general-purpose apps whatever config says.

**Dev builds** (`Source::DevBuild`, `via: "dev-build"`): when the captured app's executable (the front app's pid, or a Window capture's window owner, through `proc_pidpath`; Hex can send `pid` or `executable`) sits inside a git working tree, that tree's newest open session gets it, like a localhost port, after the rules. `policy::dev_build_root` checks at most 12 parents for `.git` and never takes the home folder; apps in `/Applications` and `/System` aren't in one. A general-purpose app built from source gives nothing.

**Rule offers** (`policy::learn`): each pick of a chip, and each send left to stand, counts the session's repository (else its project) for the capture's site (its tab's origin; loopback ones never) or app (its bundle ID; never a general-purpose app) in `~/Library/Application Support/scrawl/router/learned.json`. When two picks in a row agree, the result carries `offerRule: {"kind": "site" | "app", "key", "folder"}`: a capture's `routing.json` gets it, and `scrawl router deliver` prints it as `offer_rule`. A send a rule, port or dev build already made teaches nothing, and a key that already has a rule or is in `routing.refusedRules` is never offered. The capsule's ⌥⇧V list draws the offer. `scrawl router rule add --site <URL prefix> | --app <bundle ID> --folder <folder or ses_…>` writes the rule into config.json (other keys and `0600` kept) and lifts a refusal; `scrawl router rule refuse <key>` adds the key to `routing.refusedRules`.

A 429 from the gateway is retried once after 400 ms (or a `Retry-After` up to 1 s) when the time left allows. Any other failure (no key, network, a bad answer, over 3 s) leaves smart routing's decision standing and says why in `routing.json`'s `jev` section and the log. The key comes from `"jevApiKey"`, then `$OPENCODE_API_KEY`, then OpenCode's own credential (read-only), and goes to curl on stdin, never in arguments or logs. `scrawl route --explain <bundle> --jev` asks again about one capture.

**Named at the start** (`routing::named`, `via: "named"`): when what was said (a capture's transcript, Hex's text) opens by addressing a session, it's sent there with the usual countdown, ahead of a site rule and without asking Jev. The addressing phrases are a greeting (`hey hex …`), `tell`/`ask`/`ping <name> …`, `for`/`to <name>:` and `<name>,` or `<name>:`, after dropping filler ("okay", "um", "so") and an optional "the" and trailing "session"/"agent". Only the opening counts: "scroll down" or "fix the scrawl bug" is never an address. The name, up to three words, matches an open session's title exactly, then its folder's name, then a title or folder it begins (3+ letters), then one edit away (4+ letters), letters and digits only, so "voice control" is `voice-control`. Two different names matching as well is no address; one name on several sessions picks the newest. `"routing": {"aliases": {"scroll": "SCRAWL"}}` maps spoken words to a title or folder, matched exactly; the repo has none built in. The words stay in what's delivered.

### `scrawl router`

Hex calls `scrawl router decide` for dictation. It reads, on stdin:

```json
{"item": {"item_id": "…", "kind": "capture" | "dictation", "text": "…", "app": "…" | {"id": "…", "name": "…"},
          "title": "…", "url": "…", "selected_text": "…", "mode": "…",
          "windows": [{"app": "…", "title": "…"}], "focused": {"role": "…", "title": "…", "value_excerpt": "…"},
          "visible_text": "…", "opencode_tab": "…", "recent": [{"text_excerpt": "…", "app": "…", "routed_to": "ses_…"}]},
 "pinned": ["ses_…"],
 "history": [{"text": "…", "to": "ses_…", "corrected_to": "ses_…"}]}
```

Only `text` is required (`transcript` and `window_title` work as aliases; `kind` defaults to `dictation`; unknown fields are ignored; `selected_text` is cut to 2000 characters). `item_id`, the caller's own ID, is echoed in the reply and the log. An app given only as a bundle ID is named by its last part. `pinned` sessions are always put to Jev but don't win on their own. `history` (the latest 10) goes to Jev as state, each session named by its target when it's one, and the sessions it names join the targets. The URL, title and app go through the same rules and clues as a capture, so a `routing.sites` rule or a signed page still decides without Jev. It prints, within 3 s:

```json
{"decision_id": "19a1f…", "item_id": "…",
 "decision": "send" | "ask" | "fallback" | "keep",
 "target": {"id", "title", "directory"} | null,
 "candidates": [{"id", "title", "directory", "score"}],
 "confidence": 0.95, "any_fits": 0.9, "model": "jev-1.13", "ms": 640,
 "why": "jev: confident"}
```

`candidates` are Jev's probabilities, best first: for `ask` the chips, otherwise up to 5. A suggestion (`"routing": {"autoSend": "rules"}`, the default) adds `"suggested": true` and `"would": "send"` or `"fallback"`, and `why` reads `jev: would send; suggested (autoSend rules)`. When Jev wasn't asked (a rule, a signed page, a port, a publisher, a dev build, or Jev failing), `confidence`, `any_fits`, `model` and `ms` are null (`ms` is set when Jev failed), `candidates` are what smart routing would show, unscored, and `why` names the source: `named at the start`, `routing.sites rule`, `routing.apps rule`, `dev build's checkout`, `signed page`, `localhost port`, `publisher marker`, or smart routing's reason with Jev's error. `item.pid` or `item.executable` (an absolute path) lets a dev build decide. Reading sessions and asking Jev share 2.3 s; past that, or past 2.5 s overall, the answer is `keep` with `why: "timeout"`. For dictation `keep` means nothing fits, so Hex shows its picker; for a capture it means the clipboard only. `decide` never sends: it logs the decision and saves the request and reply to `~/Library/Application Support/scrawl/router/decisions/<decision_id>.json` (`$SCRAWL_ROUTER_DIR`), kept for 7 days.

`scrawl router deliver --decision <id> --to <ses_…>`, or `--item-id <id> --session <ses_…>`, sends a saved decision and prints `{"ok", "state": "sent" | "queued", "session", "offer_rule"?}`, plus `"delivery_id", "message_id", "delivery", "was_idle"` once sent (see Undo). Text on stdin replaces the decision's (and is enough on its own with `--item-id`). Dictation goes as a plain prompt (`{"text", "delivery": "steer"}`, nothing else marking who wrote it) whose first line says where it came from, `[voice via Hex · from Ghostty]` (without `· from` when the app isn't known, and not added twice when the text already starts with it), with the decision's selection quoted after it unless stdin replaced the text; a capture whose request named its `bundle` sends the bundle the usual way. A failed send is `queued`: the prompt waits in `router/outbox/<id>/` with a `.deliver.json`, and the app (the port owner) retries it on the same schedule as a bundle.

Corrections come two ways, through one writer that checks the schema below: `scrawl router correct --decision <id> --action cancel|redirect|paste|edit [--to <ses_…>]` (from the decision's target to `--to`, required for `redirect`), or `scrawl router log` with one line on stdin, `{"kind", "event": "correction", "item_id" or "decision_id", "action", "from"?, "to"?}`, where `from` and `to` may be bare session IDs. A logged line is joined to its decision by `decision_id` or `item_id`, which fills in the other ID, `from` (the decision's target), titles, `app` and `time`. The app logs its own captures' corrections through the same writer.

`scrawl router eval [--since 7d] [--limit 40] [--lines] [--no-screen]` replays past captures whose real destination is known (a chip pick, or the session that received the bundle) and prints accuracy by decision. It then asks about each line of `router/eval/*.jsonl`, `{"item": {…}, "expect": {"to": "ses_…"} | {"decision": "fallback"}}`, and reports them as their own rows by file and line; those files are Kit's and stay out of the repo. Last come two tables over both: for the confidence and margin gates, what each value sends and how many right and wrong sends it blocks; and how many right sends `any_fits` alone would have demoted at each value, with the rate, by kind. An `autoSend` line follows the bands: how many sends and fallbacks Jev would make (right and wrong), how many the setting turns into suggestions with the pick pre-selected (right first), and how many still go on their own. Items are judged on what Jev would have done, with their suggestions counted beside. "Truth on a chip" counts the chips a decision shows: Jev's ask up to four, a send its one switch, smart routing's own offer two. `--lines` puts the targets' recent lines in whatever the config says and `--no-screen` leaves the `screen` out, to compare runs.

#### Undo

Every send to a session is remembered for 7 days in `router/deliveries/<delivery_id>.json` (mode 0600): `router deliver`'s, the app's capture sends and both kinds of retry. A sent `router deliver` adds:

```json
{"delivery_id": "19a1f…", "message_id": "msg_…" | null, "delivery": "steer" | "queue", "was_idle": true | false | null}
```

`message_id` is the inbox item's ID, which the message keeps once delivered; `was_idle` is whether the session had no turn running just before (`GET /api/session/active`; null when that check failed). A queued send has none of these until a retry sends it, so it can't be undone from Hex.

`scrawl router undo <delivery_id> [--json]` does the least that takes the send back:

- `withdrawn`: still in the session's inbox, so it's cancelled there.
- `reverted`: it started a turn (the session was idle and it's the first thing said since) and nothing was said after it, so the turn is interrupted (`resume=false`) and a revert to it is staged with its file changes, not committed, so `/redo` brings it back.
- `disowned`: it joined running work, more came after it, or the revert couldn't be staged, so it steers "Ignore my previous message: it was sent to this session by mistake." Nothing is aborted or reverted.
- `gone`: no record, or the session or message no longer exists.
- `too-old`: sent more than `routing.undoWindowSec` (default 600) ago; nothing is done.

It prints `{"undone": "withdrawn" | "reverted" | "disowned" | "gone" | "too-old", "session": "ses_…" | null, "message_id": "msg_…" | null}` and exits 0, or nonzero for a malformed ID or a server it can't reach. Asking again answers the same without touching the session. An item delivered between the inbox read and the cancel (which then does nothing) is found among the messages and reverted or disowned. The undo is logged as a correction, `{"event": "undo", "action": "cancel", "undone", "delivery_id", "from": the session, "to": null}`, joined by `decision_id` and `item_id` (or `bundle`), and takes back the pick the send counted toward a rule offer; an undone capture's `routing.json` gets `undone` and `eval` leaves it out.

### The decisions log

`~/Library/Application Support/scrawl/routing-decisions.jsonl` (`$SCRAWL_ROUTING_LOG`), one JSON object per line, written only with Jev routing on (and by every `scrawl router decide`):

| Field | |
|---|---|
| `time` | epoch milliseconds |
| `kind` | `capture` or `dictation` |
| `event` | `decision`, `correction` or `undo` (`scrawl router undo`) |
| `bundle` / `decision_id`, `item_id` | the bundle folder's name (captures), or `scrawl router decide`'s decision ID and the caller's item ID |
| `decision` | `send`, `ask`, `fallback` or `keep` (decisions) |
| `by` | `jev`, `smart` when smart routing's own decision stood, or `timeout` |
| `to` | `{"id", "title"}` or null: where it was sent or first offered; for a correction, where Kit put it (null when he kept it or cancelled) |
| `from` | `{"id", "title"}` or null: what routing had suggested (corrections) |
| `action` | `cancel`, `redirect`, `paste` or `edit` (corrections) |
| `detail` | in the app: `escape`, `dismiss` (✕) or `paste` during the countdown, `chip`, or `reroute` (`scrawl send --reroute`) |
| `confidence`, `any_fits` | Jev's numbers, or null |
| `model`, `ms` | the Jev model and the call's time, or null |
| `app` | the app the item came from |

A redirect whose `from` and `to` match (picking the suggested chip) is a confirmation.

## Laws

The tests in `smart.rs` and `look.rs` check each one. An address always wins, and a choice beats a recorded destination. A send needs `SEND_AT` from clues and a lead of `LEAD`. `decide` is pure, so the same evidence gives the same decision, in any order and with clues repeated. Recency never puts a session forward on its own and never reaches `SEND_AT`. Adding support for the leader never demotes it. The home folder and `/` are never projects. One medium clue (the viewed session, a terminal folder, the app's checkout or name) can't send without a second that agrees. `look::tests::the_ladder_policy_reproduces_route` checks that `Policy::Ladder` over the same hooks decides what `routing::route` does on a table of cases.

Real captures replay as fixtures in `src/routing/fixtures/` (sessions trimmed): the OpenCode capture that was sent to OPENCODE IOS while Kit was talking about OpenCode's composer now asks about DESKTOP with OPENCODE IOS one click away; the same capture talking about the session suggests OPENCODE IOS; a dev site and two Device Hub captures decide as they did. Three gallery pages (`artifact-*.json`, with the file's path and time and the tool calls history held) that went to CANVAS on the port go to HEX, HEX and GAME ENGINE, who wrote them; without the history the two Hex pages ask about HEX and the third stays on the clipboard. A capture of the onboarding story's own server (`artifact-server-onboarding.json`) that went to HOME goes to KNOBS, who wrote most of it; without the history it isn't sent to HOME.

## The pill

The Metal capsule (`src/native/pill`) draws the chips as `ElemId::Route0`/`Route1` from `Pres::Completed { label, routes }`, with labels rasterized into reserved atlas rows (`GlyphAtlas::update_route`) and reported as `PillAction::Route(i)`. A chip inserted between items that are still travelling waits until they've cleared its spot before it fades in. `native::pill::tests::route_chips_render_offscreen` draws every state offscreen; with `SCRAWL_PILL_STILLS=DIR` it writes them, and frames of the morph, as PNGs. The GPUI capsule shows the same chips. The SwiftUI pill (`feat/swiftui-pill`, `helper/Pill.swift`) would need `route:0`/`route:1` items with their labels in the state it's sent, drawn like `inspect` (the first with a faint fill when it reads "Send to …"), and to report a `route:<i>` action.

## Tools

- `scrawl route --explain <bundle|latest> [--url URL]` prints the evidence, the sessions weighed with their points, the decision with why it wasn't sent and what's one click away, and what the ladder does. It replays `routing.json` when the bundle has one, keeping the sessions and what was seen live (the tab, the port, the publisher, the terminal) and reading the bundle's own clues again with today's rules; a page from a file server saved before publishers were looked for is looked at now. Otherwise it gathers what it can now (sessions as they are today, so recency and the viewed session are only approximate), and `--url` stands in for the tab.
- With Jev routing, `--explain` also prints the `jev` section saved at capture time, and `--jev` asks Jev again (one call). `scrawl router decide|deliver|correct|eval` are described under [Jev](#jev).
- Tests swap the outside steps through `look::Senses` (which holds `routing::Hooks`), and never post to a session.

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

- `"routing": {"apps": {"<bundle ID or prefix>": "<folder>"}}` in config.json, for apps whose checkout is named otherwise (`"com.kitlangton.problems": "~/code/worktrees/team-board/rewrite"`). The longest matching prefix wins.
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

## Laws

The tests in `smart.rs` and `look.rs` check each one. An address always wins, and a choice beats a recorded destination. A send needs `SEND_AT` from clues and a lead of `LEAD`. `decide` is pure, so the same evidence gives the same decision, in any order and with clues repeated. Recency never puts a session forward on its own and never reaches `SEND_AT`. Adding support for the leader never demotes it. The home folder and `/` are never projects. One medium clue (the viewed session, a terminal folder, the app's checkout or name) can't send without a second that agrees. `look::tests::the_ladder_policy_reproduces_route` checks that `Policy::Ladder` over the same hooks decides what `routing::route` does on a table of cases.

Real captures replay as fixtures in `src/routing/fixtures/` (sessions trimmed): the OpenCode capture that was sent to OPENCODE IOS while Kit was talking about OpenCode's composer now asks about DESKTOP with OPENCODE IOS one click away; the same capture talking about the session suggests OPENCODE IOS; a dev site and two Device Hub captures decide as they did. Three gallery pages (`artifact-*.json`, with the file's path and time and the tool calls history held) that went to CANVAS on the port go to HEX, HEX and GAME ENGINE, who wrote them; without the history the two Hex pages ask about HEX and the third stays on the clipboard. A capture of the onboarding story's own server (`artifact-server-onboarding.json`) that went to HOME goes to KNOBS, who wrote most of it; without the history it isn't sent to HOME.

## The pill

The Metal capsule (`src/native/pill`) draws the chips as `ElemId::Route0`/`Route1` from `Pres::Completed { label, routes }`, with labels rasterized into reserved atlas rows (`GlyphAtlas::update_route`) and reported as `PillAction::Route(i)`. A chip inserted between items that are still travelling waits until they've cleared its spot before it fades in. `native::pill::tests::route_chips_render_offscreen` draws every state offscreen; with `SCRAWL_PILL_STILLS=DIR` it writes them, and frames of the morph, as PNGs. The GPUI capsule shows the same chips. The SwiftUI pill (`feat/swiftui-pill`, `helper/Pill.swift`) would need `route:0`/`route:1` items with their labels in the state it's sent, drawn like `inspect` (the first with a faint fill when it reads "Send to …"), and to report a `route:<i>` action.

## Tools

- `scrawl route --explain <bundle|latest> [--url URL]` prints the evidence, the sessions weighed with their points, the decision with why it wasn't sent and what's one click away, and what the ladder does. It replays `routing.json` when the bundle has one, keeping the sessions and what was seen live (the tab, the port, the publisher, the terminal) and reading the bundle's own clues again with today's rules; a page from a file server saved before publishers were looked for is looked at now. Otherwise it gathers what it can now (sessions as they are today, so recency and the viewed session are only approximate), and `--url` stands in for the tab.
- Tests swap the outside steps through `look::Senses` (which holds `routing::Hooks`), and never post to a session.

# Releasing Scrawl

Merge to `main` → cut a build as a beta → Kit uses it → promote that exact build to stable.

Every build is a candidate: the app carries a plain version and an increasing build number
(`0.3.14 (18)`), never the word "beta". Beta is a property of the build's appcast item
(`<sparkle:channel>beta</sparkle:channel>`) and of the user's `"updateChannel": "beta"` setting.
Promotion removes that one element, so each build has exactly one appcast item and stable users
get the bytes beta users tested. The site, its download aliases, agent files and feature map
follow the newest stable item only.

| | Beta build | Stable (promoted) |
| --- | --- | --- |
| App says | `0.3.14 (18)` | `0.3.14 (18)` (same bytes) |
| GitHub | prerelease `v0.3.14-b18`: `Scrawl-0.3.14-b18-arm64.dmg/.zip`, `…-windows-x64.zip` | latest `v0.3.14`: the same files as `Scrawl-0.3.14-*` |
| Appcast | item for build 18 with `sparkle:channel` beta | the same item, channel removed |
| Homebrew | `scrawl@beta` → `0.3.14-b18` | `scrawl` → `0.3.14` |

## Cut a beta

On **Pro** (the release host), in a clean checkout of merged `main`:

```sh
./scripts/release-beta.sh --dry-run
./scripts/release-beta.sh                    # the next build of the version in flight
./scripts/release-beta.sh --version 0.3.15   # start a new version
```

Without `--version` the build belongs to the newest version that has builds or tags. Once that
version is stable the script stops and asks for `--version`. The build number is one more than
every appcast item, archived build (failed ones included) and `-bN` tag. Re-running at the same
commit resumes the same build; commit something to cut another. `Cargo.toml`'s version is only
the development default: release builds get theirs from `SCRAWL_VERSION`.

The script reserves the build in `~/.local/share/scrawl-release/builds/<build>/release.json`, then
runs `release-app.sh` in a one-off GUI-session launchd job (the login keychain signs): HiveMind
builds with `SCRAWL_VERSION`/`SCRAWL_BUILD_NUMBER`, fetches both binaries back and checks
`scrawl --version` says this build; then Developer ID signing, notarization and stapling. The DMG
and Sparkle zip are archived with their SHA-256s; retries reuse those exact bytes and refuse
changed ones. Keep `builds/` backed up.

It tags `v<version>-b<build>`, publishes a GitHub **prerelease** with the DMG and zip (the tag
starts `.github/workflows/release-windows.yml`, which attaches `Scrawl-<version>-b<build>-windows-x64.zip`
and never replaces an uploaded zip), copies the zip into Pro's `updates/` and asks the
`dev.kit.scrawl-appcast` job for a beta item:

```sh
generate_appcast --ed-key-file - --versions <build> --channel beta --maximum-versions 0 \
  --download-url-prefix https://scrawl.kitlangton.com/releases/ <updates>
```

`--versions` adds only this build; `--maximum-versions 0` keeps every older item and archive (deltas
need them). The job reads the ed25519 key with `security`, as before, backs the feed up to
`feed-backups/`, and restores it if anything is off: a build with more than one item, another
item's channel changed, an archive moved out of `updates/`, or this build's enclosure unsigned
(what `generate_appcast` does when the key doesn't match the app's `SUPublicEDKey`). The first run
saves the old `appcast.sh` as `appcast-before-channels.sh` and installs `scripts/appcast-job.sh`
and this script; requests go through `appcast-request.json`, the log is `appcast.log`.

Then it deploys the site from Pro (Wrangler's login is there) through skulk's heavy slot, after
fetching the Mac and Windows downloads of every `vX.Y.Z` and `-bN` release, so a fresh checkout
drops none (other releases, like `linux-preview-N`, stay on GitHub; files over Workers Assets'
25 MiB limit stop the deploy). `Scrawl-latest-*` stays on
stable. The `scrawl@beta` cask PR updates (the first beta updates draft PR
kitlangton/homebrew-tap#39). Kit merges cask PRs.

Before promoting, wait for the Windows workflow and look at its fresh-profile screenshot. On macOS
check startup, permissions, capture, paste, and update checks with beta updates on and off, and
the changed feature on real captures.

## Promote a tested build

```sh
./scripts/promote.sh 18 --dry-run
./scripts/promote.sh 18
```

It checks the archived hashes, tags `v<version>` on the build's commit (refused if that version is
already stable as another build), and publishes a GitHub release with the same DMG and zip named
`Scrawl-<version>-*`, plus the Windows zip (and a Linux one, once Linux builds attach theirs) copied from the `-b<build>` prerelease
with a fresh `.sha256`. Stable tags don't run the Windows workflow. It refuses until the Windows
zip exists, and refuses to promote behind a newer stable build.

The appcast job removes the build's `<sparkle:channel>` line and nothing else; nothing is
regenerated or re-signed. The site deploys with `Scrawl-latest-*` on the new version, GitHub marks
the release latest, and a `scrawl` cask PR opens.

In the appcast PR, change `"status": "main"` entries in `site/src/features.json` to
`"stable@<version>"` for features in the promoted commit. Later main-only work stays `main`.

## What users get

Sparkle drops items whose channel isn't allowed (`allowedChannelsForUpdater:` in `src/sparkle.rs`
returns `beta` only with the setting on; items without a channel are always allowed), then offers
the newest item only if its `CFBundleVersion` is higher than the running one. So:

- Stable users never see a beta item.
- Beta users get every new build, beta or stable. A promotion offers them nothing new: they
  already run that build.
- Turning beta off never downgrades: Scrawl stays on its build until a stable one with a higher
  build number arrives.
- When two builds share a version, Sparkle's alert shows both build numbers ("0.3.14 (19) — you
  have 0.3.14 (18)").

## Recovery and checks

`--dry-run` reads the live appcast, the archive and GitHub (REST) and prints the plan and anything
that would stop it; it writes nothing, fetches no git refs and starts no jobs. Promotion dry runs
need the archive, so run them on Pro. One release runs at a time (`release.lock`). A retry
accepts a dirty `site/public/appcast.xml` only when it matches `updates/appcast.xml`. After
publishing, commit `site/public/appcast.xml` in an appcast PR.

Never delete or replace `~/.local/share/scrawl-release/updates/`, and keep Sparkle's tools and the
archive out of `target/` and `/tmp`. `setup-sparkle.sh` checks the pinned Sparkle download.
`release-app.sh` only builds, signs and notarizes; publish through the two scripts so every build
shares one build-number sequence.

After deploying, check the live appcast and download hashes, the Sparkle signature, and `spctl`
on the downloaded DMG and app; that the site's Mac and Windows buttons, `llms.txt` and the feature
map show the stable version; and that the Windows zip has `scrawl.exe`, the transcription and CPU
backend DLLs, the Visual C++ runtime DLLs, README and LICENSE (see [windows.md](windows.md)).

```sh
python3 -m unittest discover -s scripts -p test_release_channel.py
bun test site/tests/appcast.test.ts
hivemind run --kind cargo -- cargo test -- updates:: settings::tests::toggles_and_popups_write_their_keys
```

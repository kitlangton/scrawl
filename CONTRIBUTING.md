# Contributing to Apparent

Found a bug or have an idea? [Open an issue](https://github.com/tryapparent/apparent/issues/new). A short Apparent recording is especially useful: show the problem, talk through what you expected, and point at the part that went wrong. It helps us understand the issue *and* see where Apparent itself could be better.

## Report an issue with a recording

1. Record the problem with Apparent. On macOS press **⌥⇧S** to start and stop; on Linux press **Alt+Shift+S**. Reproduce the issue, narrate what you expected, and mark anything easy to miss with Shift.
2. Review the recording **before sharing it publicly**. Check the video, audio, contact sheet, frames, and transcript for passwords, tokens, private messages, customer data, or other sensitive details. If it contains any, record a clean reproduction instead; do not upload the raw `.apparent` bundle.
3. Run `apparent export latest --out issue.mp4` and attach the reviewed MP4 to the issue. Export works on macOS and Linux (Linux requires `ffmpeg` and `ffprobe`), with smooth activity zoom by default; add `--no-zoom` for a static camera. If you cannot export a video, attach only reviewed, safe screenshots or describe the steps in text. A recording is welcome, never required.
4. In the issue, add what you expected, what happened, how to reproduce it, and your OS and Apparent version. If the issue is *about Apparent*, show where recording or handoff became confusing too—that feedback is part of the report.

If you cannot safely share a recording, a text-only issue is just as welcome. Do not post private capture paths or an unreviewed `summary.md`; the transcript can contain things you said aloud even when they are not visible in a frame.

## Contribute code

Before a larger change, open an issue to discuss the approach. For a pull request, explain the behavior you changed and how you tested it. See [development reference](enablement/docs/development.md#testing) for the project test commands and [AGENTS.md](AGENTS.md) for repository-specific build and safety rules.

## Guard local pushes

From the checkout you push, run `git config extensions.worktreeConfig true` once for the repository, then `git config --worktree core.hooksPath "$(git rev-parse --show-toplevel)/.githooks"`. This enables the tracked pre-push hook **for this worktree only**. Each push of a commit runs the offline Cargo build and full test suite, and rejects a dirty checkout, a ref that is not its checked-out `HEAD`, or a failed check. A ref deletion needs no build. Install it separately in each worktree; Git does not install tracked hooks automatically. The hook uses `taskpolicy` if available, or plain Cargo with the same job limit otherwise; its offline dependencies must already be installed.

This is a local safety rail, not server enforcement: `git push --no-verify`, another checkout without the hook, or a direct API write can bypass it. Protect the branch with required CI checks on GitHub if you need a rule that applies to everyone.

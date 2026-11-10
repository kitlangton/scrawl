# Staged directory-width guard

`.githooks/pre-commit` runs `check.py` against the Git index tree (`git write-tree`), never mutable worktree contents. Eligibility is by first-party tree scope, not filename suffix: every tracked blob outside the narrow dependency/cache and named generated-output exclusions below makes its containing directory and ancestors eligible. Thus extensionless and uncommon-suffix source files, docs, and assets in first-party directories cannot evade the guard by naming alone. It checks changed eligible directories and directories whose immediate entries changed, including the repository root when a top-level entry changes. Editing any immediate first-party root file also checks root width. Ordinary nested edits do not police unrelated over-width ancestors. Files and subdirectories have independent limits of 10. Symlinks count as files and gitlinks as directory entries; neither is followed.

Only dependency/cache trees (`target`, `node_modules`, `vendor`, `third_party`, `third-party`, `.git`, `__pycache__`) and specifically generated roots `public/site/published-homepage`, `public/site/video-demo/output`, `connect/web/dist`, and `connect/web/dist-site-check` are excluded. `public/site/public` is not excluded: tracked feature-demo HTML there is first-party code. Generic `src`, `shared`, library, test, `dist`, and generated-named directories receive no blanket exemption. On violation, split changes by responsibility and stage only the intended changes; the hook never edits or stages files. Git/plumbing failures reject the commit.

At root, `.cargo`, `.githooks` and `.github` are the three bounded tool-directory
exceptions described in `AGENTS.md`. Root permits ten ordinary directories plus
those exact tool directories; all remain visible in the actual count. Their
contents and every nested directory retain the ordinary ten-entry limits.
Other hidden directories and owner roots (including `record` and `onboarding`)
count normally. A symlink remains a file; a gitlink remains an ordinary directory
and grants no tool allowance. Counts come only from actual staged Git trees, and
root checks print total, tool and ordinary counts even on success.
Root files still have a separate limit of ten. This changes no
build, native acceptance, verification selector or release gate.

Run focused integration coverage with:

```sh
python3 -m unittest discover -s enablement/directory-width/tests -v
```

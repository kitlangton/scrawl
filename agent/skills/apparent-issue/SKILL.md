---
name: apparent-issue-reporting
description: Prepare and, only with explicit human approval, submit a text-only Apparent bug report or improvement request.
---

# Report an Apparent issue

Use `apparent-agent issue` to prepare a public issue in `tryapparent/apparent`. This submits feedback only: it does not implement a change, contact a person, or merge work.

## Preview first

Search all existing issues before drafting so an existing report can be referenced instead of duplicated:

```sh
gh issue list --repo tryapparent/apparent --state all --search 'distinctive symptom'
```

Prepare exact title and body text without `--submit`:

```sh
apparent-agent issue --title 'Short issue title' --body-file './issue draft.md'
```

The command prints JSON with `repository`, `title`, `body`, and `submitted: false`. Show the human the exact repository, title, and complete body from that JSON. Ask for explicit approval of those exact values before submission. If any text changes, preview and approve the changed text again.

## Submit only approved text

After approval, rerun the same command with `--submit`:

```sh
apparent-agent issue --title 'Short issue title' --body-file './issue draft.md' --submit
```

This creates a public issue using the local `gh` login, pins the service and repository, and reads the issue back to verify URL, number, title, and body. Success JSON is printed only after that verification. If create or readback fails, keep the failure status, use any known URL to inspect the issue, and search the repository before deciding what to do next. Never blindly retry: an ambiguous failure may have created the issue.

Do not attach or automatically add captures, transcripts, recordings, local paths, config, credentials, personal data, logs, or diagnostics. The tool collects no such material. Include only text the human approved for public posting. Validation does not redact private information. The preview is local and needs no GitHub login; submission requires installed `gh` authentication for github.com and never prompts, installs software, opens a browser, or retries.

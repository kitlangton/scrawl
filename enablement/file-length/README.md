# Staged file-length guard

The pre-commit hook rejects added, modified, renamed or type-changed regular text
files with more than **2,000 physical lines**, including comments and blank lines.
A final unterminated line counts. It reads staged Git blobs, so partial staging
is respected. Untouched oversized files and deleted files do not block commits;
changed oversized files must meet the limit even when shrinking. It never edits
or stages files. Git errors and unresolved index entries fail closed.

Exclusions: `*.lock` (including Cargo, Yarn and Bun locks),
`package-lock.json`, `npm-shrinkwrap.json`, `pnpm-lock.yaml`, `bun.lockb`; dependency/cache path
components `.git`, `target`, `node_modules`, `vendor`, `third_party`, `third-party`,
`__pycache__`; generated roots `public/site/video-demo/output`, `connect/web/dist`
and `connect/web/dist-site-check`. Binary blobs (NUL in the first 8,000 bytes),
symlinks and submodules are skipped. Other text, including docs, JSON, SVG and
extensionless scripts, is checked. A generated-looking filename alone is not an
exemption. The existing directory-width check still applies independently.

Requires Node 22.6+ with built-in TypeScript stripping (the repository uses Node
22). Run the focused real-Git hook tests with:

```sh
node --experimental-strip-types --test enablement/file-length/check.test.ts
```

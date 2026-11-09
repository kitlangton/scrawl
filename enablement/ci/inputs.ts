// Committed input facts shared by local and hosted selectors; plans stay separate.
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const options = (cwd: string) => ({cwd, encoding: 'utf8' as const, maxBuffer: 16 * 1024 * 1024});
export const controlWorkflows = ['.github/workflows/checks.yml', '.github/workflows/stream-public.yml'];

export function changedPaths(base: string, head: string, cwd = root): string[] {
  // Both sides of moves/deletions count; never compare only the latest commit.
  return execFileSync('git', ['diff', '--no-renames', '--name-only', '-z', base, head, '--'], options(cwd))
    .split('\0').filter(Boolean);
}

export function workflowControlPaths(base: string, head: string, paths: string[], cwd = root): string[] {
  const candidates = paths.filter(path => controlWorkflows.includes(path));
  if (!candidates.length) return [];
  try {
    const entries = execFileSync('git', ['diff', '--raw', '--no-abbrev', '--no-renames', '-z',
      base, head, '--', ...candidates], options(cwd)).split('\0');
    const snapshots: {path: string; before: string; after: string}[] = [];
    for (let index = 0; index < entries.length - 1; index += 2) {
      const match = /^:100644 100644 ([a-f0-9]{40}) ([a-f0-9]{40}) M$/.exec(entries[index]);
      const path = entries[index + 1];
      // Adds/deletes/type/mode changes cannot supply the required preservation proof.
      if (!match || !candidates.includes(path)) continue;
      const blob = (oid: string) => new TextDecoder('utf-8', {fatal: true, ignoreBOM: true}).decode(
        execFileSync('git', ['cat-file', 'blob', oid], {cwd, maxBuffer: 16 * 1024 * 1024}));
      snapshots.push({path, before: blob(match[1]), after: blob(match[2])});
    }
    // Bun's existing YAML parser is also used by the hosted workflow contracts.
    // Missing tools, unreadable blobs and unrecognised YAML always retain coverage.
    return JSON.parse(execFileSync('bun', [fileURLToPath(new URL('./workflow-controls.ts', import.meta.url))],
      {...options(cwd), input: JSON.stringify(snapshots)}));
  } catch { return []; }
}

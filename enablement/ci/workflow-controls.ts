import {YAML} from 'bun';
import {isDeepStrictEqual} from 'node:util';
import {readFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';

// Recognise only the ordinary root block used by the two audited orchestrators.
// Byte equality outside it complements parsed equality: aliases, block scalars,
// duplicate/hidden execution keys and YAML normalisation cannot hide changed input.
function document(source: string): {execution: unknown; remainder: string; concurrency: unknown} | null {
  if (/[\r\t\0\uFEFF]/.test(source) || /^(?:---|\.\.\.|%)/m.test(source)) return null;
  const lines = source.split('\n');
  const positions = lines.flatMap((line, index) => line === 'concurrency:' ? [index] : []);
  if (positions.length !== 1) return null;
  const start = positions[0];
  let end = start + 1;
  while (end < lines.length && /^(?:  |#|$)/.test(lines[end])) end++;
  const fields = lines.slice(start + 1, end).filter(line => line.trim() && !/^\s*#/.test(line));
  if (fields.length !== 2 || !fields.every(line => /^  (group|cancel-in-progress): \S/.test(line))) return null;
  if (new Set(fields.map(line => line.split(':')[0])).size !== 2) return null;
  // No anchors/aliases/tags, flow containers or multiline values in the control
  // block: a new anchor could otherwise change an unchanged execution alias.
  if (fields.some(line => /^  [\w-]+: [!&*|>\[{]/.test(line))) return null;
  let begin = start;
  while (begin > 0 && /^(?:#|$)/.test(lines[begin - 1])) begin--;
  const parsed = YAML.parse(source) as Record<string, unknown>;
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
  const {concurrency, ...execution} = parsed;
  const control = concurrency as Record<string, unknown>;
  if (!control || typeof control !== 'object' || Array.isArray(control)
    || Object.keys(control).sort().join(',') !== 'cancel-in-progress,group'
    || typeof control.group !== 'string' || !control.group.trim()
    || !(typeof control['cancel-in-progress'] === 'boolean'
      || (typeof control['cancel-in-progress'] === 'string' && /^\$\{\{[^\n]+\}\}$/.test(control['cancel-in-progress'])))) return null;
  if (!execution.on || !execution.jobs || typeof execution.jobs !== 'object') return null;
  return {execution, concurrency, remainder: [...lines.slice(0, begin), ...lines.slice(end)].join('\n')};
}

export function concurrencyOnly(before: string, after: string): boolean {
  try {
    const old = document(before), next = document(after);
    return !!old && !!next && old.remainder === next.remainder
      && isDeepStrictEqual(old.execution, next.execution)
      && !isDeepStrictEqual(old.concurrency, next.concurrency);
  } catch { return false; }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const snapshots = JSON.parse(readFileSync(0, 'utf8')) as {path: string; before: string; after: string}[];
  console.log(JSON.stringify(snapshots.filter(({before, after}) => concurrencyOnly(before, after)).map(({path}) => path)));
}

import { constants } from 'node:fs';
import { open, readdir, readFile, stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

export const MAX_MANIFEST = 64 * 1024 * 1024;
const defaultCaptures = () => process.platform === 'darwin'
  ? join(homedir(), 'Library', 'Application Support', 'apparent', 'captures')
  : join(homedir(), '.local', 'share', 'apparent', 'captures');
const expandHome = (value: string) => value === '~' ? homedir() : value.startsWith('~/') ? join(homedir(), value.slice(2)) : value;

export async function resolveCaptures(): Promise<string> {
  let configured: unknown;
  if (process.env.APPARENT_CAPTURES_DIR !== undefined) configured = process.env.APPARENT_CAPTURES_DIR;
  else {
    const configPath = join(homedir(), '.config', 'apparent', 'config.json');
    try {
      const config = JSON.parse(await readFile(configPath, 'utf8'));
      configured = config?.capturesDir;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw new Error(`Cannot read Apparent config at ${configPath}`);
    }
  }
  if (configured === undefined) return resolve(defaultCaptures());
  if (typeof configured !== 'string' || !configured.trim()) throw new Error('capturesDir must be a non-empty path');
  return resolve(expandHome(configured));
}

export async function readManifest(path: string): Promise<{ raw: Buffer; value: Record<string, any> }> {
  const manifestPath = join(path, 'apparent.json');
  let file;
  try { file = await open(manifestPath, constants.O_RDONLY | (constants.O_NONBLOCK ?? 0)); } catch { throw new Error(`Cannot open apparent.json in ${path}`); }
  try {
    const info = await file.stat();
    if (!info.isFile()) throw new Error(`apparent.json is not a regular file in ${path}`);
    if (info.size > MAX_MANIFEST) throw new Error('apparent.json exceeds 64 MiB safety limit');
    const bytes = Buffer.allocUnsafe(MAX_MANIFEST + 1);
    let offset = 0;
    while (offset < bytes.length) {
      const { bytesRead } = await file.read(bytes, offset, bytes.length - offset, offset);
      if (bytesRead === 0) break;
      offset += bytesRead;
    }
    if (offset > MAX_MANIFEST) throw new Error('apparent.json exceeds 64 MiB safety limit');
    const raw = bytes.subarray(0, offset);
    let text: string;
    try { text = new TextDecoder('utf-8', { fatal: true }).decode(raw); } catch { throw new Error('apparent.json is not valid UTF-8'); }
    let value: any;
    try { value = JSON.parse(text); } catch { throw new Error('apparent.json is not valid JSON'); }
    validateManifest(value);
    return { raw, value };
  } finally { await file.close(); }
}

function validateManifest(value: any): asserts value is Record<string, any> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid apparent.json object');
  if (value.version !== 1) throw new Error(`Unsupported bundle version: ${String(value.version)}`);
  for (const key of ['id', 'createdAtMs', 'durationMs', 'target', 'transcript', 'strokes', 'marks', 'events', 'keyframes', 'contactSheet'])
    if (!(key in value)) throw new Error(`Invalid bundle: missing ${key}`);
  if (typeof value.id !== 'string' || !value.id || !Number.isSafeInteger(value.createdAtMs) || value.createdAtMs < 0 ||
      !Number.isSafeInteger(value.durationMs) || value.durationMs < 0 || !object(value.target) ||
      !object(value.transcript) || typeof value.transcript.text !== 'string' || !Array.isArray(value.transcript.segments) ||
      !Array.isArray(value.strokes) || !Array.isArray(value.marks) || !Array.isArray(value.events) || !Array.isArray(value.keyframes) ||
      typeof value.contactSheet !== 'string') throw new Error('Invalid bundle: malformed required semantic fields');
  if (value.transcript.words !== undefined && !Array.isArray(value.transcript.words)) throw new Error('Invalid bundle: malformed transcript words');
  if (!value.marks.every((m: any) => object(m) && typeof m.id === 'string' && Number.isSafeInteger(m.t0) && m.t0 >= 0 && Number.isSafeInteger(m.t1) && m.t1 >= 0 &&
      ['circle', 'underline', 'line', 'arrow', 'tap', 'pin', 'mark', 'flag'].includes(m.shape) && typeof m.description === 'string' &&
      (m.on === undefined || m.on === null || typeof m.on === 'string') && (m.frame === undefined || m.frame === null || typeof m.frame === 'string') && Array.isArray(m.strokes)))
    throw new Error('Invalid bundle: malformed mark fields');
  if (!value.keyframes.every((k: any) => object(k) && typeof k.file === 'string' && (k.index === undefined || Number.isSafeInteger(k.index)) &&
      (k.t === undefined || Number.isFinite(k.t)))) throw new Error('Invalid bundle: malformed keyframe fields');
  if (!value.strokes.every((s: any) => object(s))) throw new Error('Invalid bundle: malformed stroke entries');
  if (!value.events.every((e: any) => object(e))) throw new Error('Invalid bundle: malformed event entries');
}
const object = (v: unknown): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);
const safeReference = (v: unknown): v is string => typeof v === 'string' && !!v && !v.includes('\\') && !v.includes('\0') &&
  !v.startsWith('/') && !/^[a-z][a-z\d+.-]*:/i.test(v) && !v.split('/').includes('..');

export async function resolveBundle(input = 'latest', captures?: string): Promise<string> {
  if (/^[a-z][a-z\d+.-]*:\/\//i.test(input)) throw new Error('URLs are not accepted; provide a local bundle path');
  if (input === 'latest') {
    const root = captures ?? await resolveCaptures();
    let names: string[];
    try { names = await readdir(root); } catch { throw new Error(`Cannot read captures directory: ${root}`); }
    const candidates: Array<{ path: string; created: number }> = [];
    for (const name of names.filter((n) => n.endsWith('.apparent'))) {
      const path = join(root, name);
      try {
        if (!(await stat(path)).isDirectory()) continue;
        const { value } = await readManifest(path);
        // The producer publishes apparent.json last, after summary and image artifacts.
        const summaryFile = await open(join(path, 'summary.md'), constants.O_RDONLY | (constants.O_NONBLOCK ?? 0));
        try {
          if (!(await summaryFile.stat()).isFile()) throw new Error('incomplete summary');
          await summaryFile.read(Buffer.alloc(1), 0, 1, 0);
        } finally { await summaryFile.close(); }
        const references = [value.contactSheet, ...value.keyframes.map((k: any) => k.file),
          ...(value.autocrop ? [value.autocrop.fullContactSheet, ...value.keyframes.map((k: any) => k.file.startsWith('frames/')
            ? `${value.autocrop.fullFrames}/${k.file.slice('frames/'.length)}` : undefined)] : [])].filter((v) => v !== undefined);
        if (!references.every(safeReference)) continue;
        for (const reference of references) if (!(await stat(join(path, reference))).isFile()) throw new Error('incomplete image artifacts');
        candidates.push({ path, created: value.createdAtMs });
      } catch { /* malformed or incomplete candidates do not shadow a completed bundle */ }
    }
    candidates.sort((a, b) => b.created - a.created);
    if (!candidates[0]) throw new Error(`No completed .apparent captures found in ${root}`);
    return resolve(candidates[0].path);
  }
  let path = resolve(input);
  if (basename(path) === 'apparent.json') path = dirname(path);
  try { if (!(await stat(path)).isDirectory()) throw new Error(); }
  catch {
    if (!isAbsolute(input) && !input.includes('/') && !input.includes('\\')) {
      const root = captures ?? await resolveCaptures();
      path = join(root, input.endsWith('.apparent') ? input : `${input}.apparent`);
    }
    if (!(await stat(path).catch(() => null))?.isDirectory()) throw new Error(`Local bundle directory not found: ${path}`);
  }
  if (!path.endsWith('.apparent')) throw new Error('Expected a .apparent directory');
  return resolve(path);
}

export async function readBundle(path: string): Promise<Record<string, any>> { return (await readManifest(path)).value; }

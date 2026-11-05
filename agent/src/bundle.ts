import { constants } from 'node:fs';
import { open, readdir, stat, lstat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

export const MAX_MANIFEST = 64 * 1024 * 1024;
const defaultCaptures = () => process.platform === 'darwin'
  ? join(homedir(), 'Library', 'Application Support', 'apparent', 'captures')
  : join(process.env.XDG_DATA_HOME && isAbsolute(process.env.XDG_DATA_HOME) ? process.env.XDG_DATA_HOME : join(homedir(), '.local', 'share'), 'apparent', 'captures');
const expandHome = (value: string) => value === '~' ? homedir() : value.startsWith('~/') ? join(homedir(), value.slice(2)) : value;

export async function resolveCaptures(): Promise<string> {
  let configured: unknown;
  if (process.env.APPARENT_CAPTURES_DIR !== undefined) configured = process.env.APPARENT_CAPTURES_DIR;
  else {
    const configPath = join(homedir(), '.config', 'apparent', 'config.json');
    try {
      const config = JSON.parse(new TextDecoder('utf-8', {fatal:true}).decode(await readRegular(configPath, 1024 * 1024)));
      configured = config?.capturesDir;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') process.stderr.write('Ignoring unreadable or invalid Apparent config; using default capture storage.\n');
    }
  }
  if (configured === undefined) return resolve(defaultCaptures());
  if (typeof configured !== 'string' || !configured.trim()) {
    if (process.env.APPARENT_CAPTURES_DIR !== undefined) throw new Error('APPARENT_CAPTURES_DIR must be a non-empty path');
    return resolve(defaultCaptures());
  }
  const selected = expandHome(configured.trim());
  if (process.env.APPARENT_CAPTURES_DIR === undefined && !isAbsolute(selected)) return resolve(defaultCaptures());
  return resolve(selected);
}

/** Bounded descriptor read; symlink files and non-regular files are refused. */
export async function readRegular(path: string, limit: number): Promise<Buffer> {
  const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
  try {
    const info = await file.stat();
    if (!info.isFile()) throw new Error('Expected a regular file');
    if (info.size > limit) throw new Error('File exceeds ' + limit + '-byte safety limit');
    const chunks: Buffer[] = []; let total = 0;
    while (total <= limit) {
      const chunk = Buffer.allocUnsafe(Math.min(65536, limit + 1 - total));
      const { bytesRead } = await file.read(chunk, 0, chunk.length, null);
      if (!bytesRead) return Buffer.concat(chunks, total);
      total += bytesRead; chunks.push(chunk.subarray(0, bytesRead));
    }
    throw new Error('File exceeds ' + limit + '-byte safety limit');
  } finally { await file.close(); }
}

export async function readManifest(path: string): Promise<{ raw: Buffer; value: Record<string, any> }> {
  let raw: Buffer;
  try { raw = await readRegular(join(path, 'apparent.json'), MAX_MANIFEST); }
  catch { throw new Error('Cannot read regular apparent.json within 64 MiB safety limit in ' + path); }
  let text: string;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(raw); } catch { throw new Error('apparent.json is not valid UTF-8'); }
  let value: any;
  try { value = JSON.parse(text); } catch { throw new Error('apparent.json is not valid JSON'); }
  validateManifest(value);
  return { raw, value };
}

function validateManifest(value: any): asserts value is Record<string, any> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid apparent.json object');
  if (value.version !== 1) throw new Error(`Unsupported bundle version: ${String(value.version)}`);
  for (const key of ['id', 'createdAtMs', 'durationMs', 'target', 'transcript', 'strokes', 'marks', 'events', 'keyframes', 'contactSheet'])
    if (!(key in value)) throw new Error(`Invalid bundle: missing ${key}`);
  if (typeof value.id !== 'string' || !value.id || !Number.isSafeInteger(value.createdAtMs) || value.createdAtMs < 0 ||
      !Number.isSafeInteger(value.durationMs) || value.durationMs < 0 || !object(value.target) ||
      !['display', 'window', 'region', 'device'].includes(value.target.mode) ||
      !Number.isSafeInteger(value.target.displayId) || !object(value.target.frame) ||
      !['x', 'y', 'width', 'height', 'scale'].every((key) => typeof value.target.frame[key] === 'number' && Number.isFinite(value.target.frame[key])) ||
      (value.target.app !== undefined && value.target.app !== null && typeof value.target.app !== 'string') ||
      (value.target.windowTitle !== undefined && value.target.windowTitle !== null && typeof value.target.windowTitle !== 'string') ||
      !object(value.transcript) || typeof value.transcript.text !== 'string' || !Array.isArray(value.transcript.segments) ||
      !Array.isArray(value.strokes) || !Array.isArray(value.marks) || !Array.isArray(value.events) || !Array.isArray(value.keyframes) ||
      typeof value.contactSheet !== 'string') throw new Error('Invalid bundle: malformed required semantic fields');
  if (value.transcript.words !== undefined && !Array.isArray(value.transcript.words)) throw new Error('Invalid bundle: malformed transcript words');
  const timedText = (item: any) => object(item) && Number.isSafeInteger(item.t0) && item.t0 >= 0 &&
    Number.isSafeInteger(item.t1) && item.t1 >= 0 && typeof item.text === 'string';
  if (!value.transcript.segments.every(timedText) || (value.transcript.words ?? []).some((word: any) => !timedText(word)))
    throw new Error('Invalid bundle: malformed transcript timing');
  if (!value.marks.every((m: any) => object(m) && typeof m.id === 'string' && Number.isSafeInteger(m.t0) && m.t0 >= 0 && Number.isSafeInteger(m.t1) && m.t1 >= 0 &&
      ['circle', 'underline', 'line', 'arrow', 'tap', 'pin', 'mark', 'flag'].includes(m.shape) && typeof m.description === 'string' &&
      (m.on === undefined || m.on === null || typeof m.on === 'string') && (m.frame === undefined || m.frame === null || typeof m.frame === 'string') && Array.isArray(m.strokes)))
    throw new Error('Invalid bundle: malformed mark fields');
  if (!value.keyframes.every((k: any) => object(k) && typeof k.file === 'string' && (k.index === undefined || Number.isSafeInteger(k.index)) &&
      (k.t === undefined || (Number.isSafeInteger(k.t) && k.t >= 0)) &&
      (k.said === undefined || typeof k.said === 'string') && (k.event === undefined || typeof k.event === 'string') &&
      (k.reason === undefined || typeof k.reason === 'string') && (k.markId === undefined || typeof k.markId === 'string')))
    throw new Error('Invalid bundle: malformed keyframe fields');
  if (!value.strokes.every((s: any) => object(s))) throw new Error('Invalid bundle: malformed stroke entries');
  if (!value.events.every((e: any) => object(e) &&
      (e.t === undefined || (Number.isSafeInteger(e.t) && e.t >= 0)) &&
      (e.kind === undefined || typeof e.kind === 'string') &&
      (e.label === undefined || e.label === null || typeof e.label === 'string') &&
      (e.text === undefined || typeof e.text === 'string')))
    throw new Error('Invalid bundle: malformed event entries');
  if (value.context !== undefined && (!object(value.context) || Object.values(value.context).some((v) => typeof v !== 'string')))
    throw new Error('Invalid bundle: malformed context');
  for (const key of ['note', 'source', 'audio', 'imageQualityWarning'])
    if (value[key] !== undefined && value[key] !== null && typeof value[key] !== 'string') throw new Error(`Invalid bundle: malformed ${key}`);
  if (value.transcript.missing !== undefined && value.transcript.missing !== null &&
      (!object(value.transcript.missing) || typeof value.transcript.missing.reason !== 'string'))
    throw new Error('Invalid bundle: malformed transcript warning');
  if (value.contextChanges !== undefined && (!Array.isArray(value.contextChanges) || !value.contextChanges.every((e: any) => object(e) && Number.isSafeInteger(e.t) && e.t >= 0 && typeof e.key === 'string' && (e.value === null || typeof e.value === 'string'))))
    throw new Error('Invalid bundle: malformed context changes');
  if (value.screens !== undefined && (!Array.isArray(value.screens) || !value.screens.every((e: any) => object(e) && Number.isSafeInteger(e.t0) && e.t0 >= 0 && Number.isSafeInteger(e.t1) && e.t1 >= e.t0 && typeof e.name === 'string')))
    throw new Error('Invalid bundle: malformed screen context');
}
const object = (v: unknown): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);
export const safeReference = (v: unknown): v is string => typeof v === 'string' && !!v && !v.includes('\\') && !v.includes('\0') &&
  !v.startsWith('/') && !/^[a-z][a-z\d+.-]*:/i.test(v) && !v.split('/').includes('..');

export async function checkImageReference(bundle: string, reference: string): Promise<void> {
  if (!safeReference(reference)) throw new Error('Bundle contains an unsafe image reference');
  let path = bundle;
  for (const part of reference.split('/')) {
    path = join(path, part);
    const info = await lstat(path).catch((error: NodeJS.ErrnoException) => { if (error.code === 'ENOENT') return null; throw error; });
    if (!info) break; // References can describe unavailable media without reading it.
    if (info.isSymbolicLink()) throw new Error('Bundle image reference follows a symlink');
  }
}

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
        // apparent.json is the producer's final publication marker. summary.md is optional
        // for local inspection because Agent can regenerate semantic text from this manifest.
        const references = [value.contactSheet, ...value.keyframes.map((k: any) => k.file),
          ...(value.autocrop ? [value.autocrop.fullContactSheet, ...value.keyframes.map((k: any) => k.file.startsWith('frames/')
            ? `${value.autocrop.fullFrames}/${k.file.slice('frames/'.length)}` : undefined)] : [])].filter((v) => v !== undefined);
        if (!references.every(safeReference)) continue;
        for (const reference of references) { await checkImageReference(path, reference); if (!(await stat(join(path, reference))).isFile()) throw new Error('incomplete image artifacts'); }
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

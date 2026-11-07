import { constants } from 'node:fs';
import { open, readdir, stat, lstat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

export const MAX_MANIFEST = 64 * 1024 * 1024;
const MAX_WAV_HEADER = 1024 * 1024;
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

/** Read only bounded WAV metadata and the first PCM16 sample, without following links. */
export async function hasMicrophoneSamples(bundlePath: string, reference: unknown): Promise<boolean> {
  if (!safeReference(reference)) return false;
  let path = bundlePath;
  try {
    for (const part of reference.split('/')) {
      path = join(path, part);
      const info = await lstat(path);
      if (info.isSymbolicLink()) return false;
      if (path !== join(bundlePath, reference) && !info.isDirectory()) return false;
    }
    const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
    try {
      const info = await file.stat();
      if (!info.isFile() || info.size < 12) return false;
      const readAt = async (offset: number, length: number): Promise<Buffer | undefined> => {
        if (offset < 0 || offset + length > info.size) return;
        const buffer = Buffer.alloc(length);
        const {bytesRead} = await file.read(buffer, 0, length, offset);
        return bytesRead === length ? buffer : undefined;
      };
      const head = await readAt(0, 12);
      if (!head || head.toString('ascii',0,4)!=='RIFF' || head.toString('ascii',8,12)!=='WAVE') return false;
      const riffEnd = head.readUInt32LE(4) + 8;
      if (riffEnd > info.size || riffEnd < 12) return false;
      let format: number | undefined, bits: number | undefined;
      for (let at = 12; at + 8 <= riffEnd && at <= MAX_WAV_HEADER;) {
        const chunk = await readAt(at,8);
        if (!chunk) return false;
        const id=chunk.toString('ascii',0,4),size=chunk.readUInt32LE(4),body=at+8;
        if (body+size>riffEnd) return false;
        if (id==='fmt ') {
          if(size<16)return false;
          const fmt=await readAt(body,16);if(!fmt)return false;
          format=fmt.readUInt16LE(0);bits=fmt.readUInt16LE(14);
          const channels=fmt.readUInt16LE(2),rate=fmt.readUInt32LE(4),bytesPerSec=fmt.readUInt32LE(8),blockAlign=fmt.readUInt16LE(12);
          if(!channels||!rate||!blockAlign||bytesPerSec!==blockAlign*rate||bits%8!==0||bits/8>Math.floor(blockAlign/channels))return false;
        } else if(id==='data') {
          const sampleBytes=bits===8?1:bits===16?2:0;
          if(format!==1||!sampleBytes||size<sampleBytes)return false;
          const sample=await readAt(body,sampleBytes);return sample!==undefined;
        }
        at=body+size+(size&1);
      }
      return false;
    } finally { await file.close(); }
  } catch { return false; }
}

export const configuredAgentContext = () => configuredExperiment('agentContext');

export async function configuredExperiment(name: 'agentContext' | 'agentDemoVideo'): Promise<boolean> {
  const fallback = name === 'agentDemoVideo';
  try {
    const raw = await readRegular(join(homedir(), '.config', 'apparent', 'config.json'), 1024 * 1024);
    const config = JSON.parse(new TextDecoder('utf-8', {fatal:true}).decode(raw));
    const experiments = config && typeof config === 'object' && !Array.isArray(config) ? config.experiments : undefined;
    if (!experiments || typeof experiments !== 'object' || Array.isArray(experiments)) return fallback;
    const v = experiments[name];
    if (typeof v === 'boolean') return v;
    if (typeof v === 'number' && Number.isFinite(v)) return v !== 0;
    if (typeof v === 'string') {
      const text = v.trim().toLowerCase();
      if (['true','yes','on','1'].includes(text)) return true;
      if (['false','no','off','0'].includes(text)) return false;
    }
  } catch { /* Invalid config is ignored, matching the local CLI's lenient read. */ }
  return fallback;
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

const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
const optionalString = (value: unknown) => value === undefined || value === null || typeof value === 'string';
const point = (value: unknown) => Array.isArray(value) && value.length === 2 && value.every(finite);
function validInteractionEvent(event: any): boolean {
  if (!object(event) || !Number.isSafeInteger(event.t) || event.t < 0) return false;
  switch (event.kind) {
    case 'note': return Number.isSafeInteger(event.pin) && event.pin >= 0 && finite(event.x) && finite(event.y) && typeof event.text === 'string';
    case 'click': return finite(event.x) && finite(event.y) && optionalString(event.button) && optionalString(event.label) && optionalString(event.role);
    case 'drag': return Number.isSafeInteger(event.t1) && event.t1 >= event.t && [event.x0,event.y0,event.x1,event.y1].every(finite) && optionalString(event.label) && optionalString(event.button) && (event.path === undefined || Array.isArray(event.path) && event.path.every(point));
    case 'scroll': return [event.x,event.y,event.dx,event.dy].every(finite) && optionalString(event.label);
    case 'point': return finite(event.x) && finite(event.y) && optionalString(event.label);
    case 'key': return typeof event.shortcut === 'string';
    case 'app_change': return typeof event.app === 'string' && optionalString(event.window_title);
    case 'app_event': return typeof event.label === 'string';
    default: return false;
  }
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
      Array.isArray(m.boxNorm) && m.boxNorm.length === 4 && m.boxNorm.every(finite) && (m.from === undefined || m.from === null || point(m.from)) && (m.to === undefined || m.to === null || point(m.to)) &&
      (m.pin === undefined || m.pin === null || Number.isSafeInteger(m.pin)) && (m.on === undefined || m.on === null || typeof m.on === 'string') && (m.frame === undefined || m.frame === null || typeof m.frame === 'string') && Array.isArray(m.strokes) && m.strokes.every(Number.isSafeInteger)))
    throw new Error('Invalid bundle: malformed mark fields');
  if (!value.keyframes.every((k: any) => object(k) && typeof k.file === 'string' && (k.index === undefined || Number.isSafeInteger(k.index)) &&
      (k.t === undefined || (Number.isSafeInteger(k.t) && k.t >= 0)) &&
      (k.said === undefined || k.said === null || typeof k.said === 'string') && (k.event === undefined || k.event === null || typeof k.event === 'string') &&
      (k.reason === undefined || typeof k.reason === 'string') && (k.markId === undefined || k.markId === null || typeof k.markId === 'string')))
    throw new Error('Invalid bundle: malformed keyframe fields');
  if (!value.strokes.every((s: any) => object(s))) throw new Error('Invalid bundle: malformed stroke entries');
  if (!value.events.every(validInteractionEvent))
    throw new Error('Invalid bundle: malformed event entries');
  if (value.context !== undefined && (!object(value.context) || Object.values(value.context).some((v) => typeof v !== 'string')))
    throw new Error('Invalid bundle: malformed context');
  for (const key of ['note', 'source', 'audio', 'imageQualityWarning'])
    if (value[key] !== undefined && value[key] !== null && typeof value[key] !== 'string') throw new Error(`Invalid bundle: malformed ${key}`);
  if (value.video !== undefined && value.video !== null && !safeReference(value.video)) throw new Error('Invalid bundle: unsafe video reference');
  if (value.videoOffsetMs !== undefined && (!Number.isSafeInteger(value.videoOffsetMs) || value.videoOffsetMs < 0)) throw new Error('Invalid bundle: malformed video clock offset');
  if (value.transcript.missing !== undefined && value.transcript.missing !== null &&
      (!object(value.transcript.missing) || typeof value.transcript.missing.reason !== 'string'))
    throw new Error('Invalid bundle: malformed transcript warning');
  if (value.contextChanges !== undefined && (!Array.isArray(value.contextChanges) || !value.contextChanges.every((e: any) => object(e) && Number.isSafeInteger(e.t) && e.t >= 0 && typeof e.key === 'string' && optionalString(e.value))))
    throw new Error('Invalid bundle: malformed context changes');
  if (value.screens !== undefined && (!Array.isArray(value.screens) || !value.screens.every((e: any) => object(e) && Number.isSafeInteger(e.t0) && e.t0 >= 0 && Number.isSafeInteger(e.t1) && e.t1 >= e.t0 && typeof e.name === 'string')))
    throw new Error('Invalid bundle: malformed screen context');
  if (value.windows !== undefined && (!Array.isArray(value.windows) || !value.windows.every((s: any) => object(s) && Number.isSafeInteger(s.t) && s.t >= 0 && Array.isArray(s.windows) && s.windows.every((w: any) => object(w) && Number.isSafeInteger(w.id) && typeof w.app === 'string' && Number.isSafeInteger(w.layer) && Array.isArray(w.bounds) && w.bounds.length === 4 && w.bounds.every(Number.isSafeInteger) && (w.regular === undefined || typeof w.regular === 'boolean') && (w.title === undefined || typeof w.title === 'string')))))
    throw new Error('Invalid bundle: malformed window context');
  if (value.autocrop?.changes !== undefined && (!Array.isArray(value.autocrop.changes) || !value.autocrop.changes.every((boxes: any) => Array.isArray(boxes) && boxes.every((b: any) => Array.isArray(b) && b.length === 4 && b.every(Number.isFinite)))) )
    throw new Error('Invalid bundle: malformed crop changes');
  for (const key of ['destination','device','autocrop']) if (value[key] !== undefined && value[key] !== null && !object(value[key])) throw new Error(`Invalid bundle: malformed ${key}`);
  if (value.destination && (typeof value.destination.session !== 'string' || typeof value.destination.via !== 'string' || value.destination.title !== undefined && typeof value.destination.title !== 'string' || !optionalString(value.destination.url) || !optionalString(value.destination.directory))) throw new Error('Invalid bundle: malformed destination');
  if (value.device && (typeof value.device.name !== 'string' || typeof value.device.capture !== 'string' || !Number.isSafeInteger(value.device.width) || !Number.isSafeInteger(value.device.height) || ['app','appVersion','appShortVersion','id','model','system'].some((k) => !optionalString(value.device[k])) || value.device.screen !== undefined && value.device.screen !== null && (!Array.isArray(value.device.screen) || value.device.screen.length !== 3 || !value.device.screen.every(finite)))) throw new Error('Invalid bundle: malformed device');
  if (value.autocrop && (typeof value.autocrop.label !== 'string' || typeof value.autocrop.fullFrames !== 'string' || typeof value.autocrop.fullContactSheet !== 'string' || !Array.isArray(value.autocrop.crops) || value.autocrop.crops.some((crop: any) => !object(crop) || !Number.isSafeInteger(crop.index) || !['x','y','width','height'].every(k => finite(crop[k])) || typeof crop.kind !== 'string' || typeof crop.reason !== 'string' || crop.apps !== undefined && (!Array.isArray(crop.apps) || !crop.apps.every((v: any) => typeof v === 'string'))))) throw new Error('Invalid bundle: malformed crop metadata');
  if (value.audio !== undefined && value.audio !== null && !safeReference(value.audio)) throw new Error('Invalid bundle: unsafe audio reference');
}
const object = (v: unknown): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);
export const safeReference = (v: unknown): v is string => typeof v === 'string' && !!v && !v.includes('\\') && !v.includes('\0') &&
  !v.startsWith('/') && !/^[a-z][a-z\d+.-]*:/i.test(v) && !v.split('/').includes('..');

export async function checkImageReference(bundle: string, reference: string): Promise<boolean> {
  if (!safeReference(reference)) throw new Error(`Bundle contains an unsafe image reference: ${reference}`);
  let path = bundle;
  const parts = reference.split('/');
  for (const [index, part] of parts.entries()) {
    path = join(path, part);
    const info = await lstat(path).catch((error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT') return null;
      throw new Error(`Cannot inspect bundle image reference ${reference}: ${error.message}`);
    });
    if (!info) return false; // References can describe unavailable media without reading it.
    if (info.isSymbolicLink()) throw new Error(`Bundle image reference follows a symlink: ${reference}`);
    if (index < parts.length - 1 && !info.isDirectory()) return false;
    if (index === parts.length - 1) return info.isFile();
  }
  return false;
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
        for (const reference of references) if (!(await checkImageReference(path, reference))) throw new Error('incomplete image artifacts');
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
  // Explicit local paths are bundles only when their bounded manifest validates;
  // unlike latest discovery, their directory name need not carry a suffix.
  await readManifest(path);
  return resolve(path);
}

export async function readBundle(path: string): Promise<Record<string, any>> { return (await readManifest(path)).value; }

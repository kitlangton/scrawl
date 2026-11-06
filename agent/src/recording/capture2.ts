import { randomBytes, randomUUID } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, mkdir, open, realpath } from 'node:fs/promises';
import { isAbsolute, join, parse, resolve, sep } from 'node:path';
import { homedir } from 'node:os';
import { performance } from 'node:perf_hooks';
import { CaptureClient, launchCaptureEngine, type CaptureTransport } from '@apparent/engine-sdk/capture';
import type { Capture2Host, Capture2Receipt } from '@apparent/record/host/capture2';

export type Capture2Clock = { now(): number; sleep(ms: number): Promise<void> };
const systemClock: Capture2Clock = { now: () => performance.now(), sleep: ms => new Promise(resolveSleep => setTimeout(resolveSleep, ms)) };
export type Capture2HostFactory = (paths: { controlRoot: string; destinationRoot: string; runtime: string; executable: string }) => Promise<{ host: Capture2Host; close(): Promise<string> }>;

async function ensurePrivateStoreRoot(controlRoot: string): Promise<string> {
  if (!isAbsolute(controlRoot) || resolve(controlRoot) !== controlRoot || controlRoot === parse(controlRoot).root) throw new Error('Agent capture/2 control root is invalid');
  const storeRoot = join(controlRoot, 'record-control-v2');
  await mkdir(storeRoot, { mode: 0o700 }).catch((error: NodeJS.ErrnoException) => { if (error.code !== 'EEXIST') throw error; });
  const parts = storeRoot.split(sep).filter(Boolean);
  let current = parse(storeRoot).root;
  for (const [index, part] of parts.entries()) {
    current = join(current, part);
    const info = await lstat(current);
    const final = index === parts.length - 1;
    if (!info.isDirectory() || info.isSymbolicLink() || (final && (info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0)))
      throw new Error('Record capture/2 store must be a private, non-aliased directory');
  }
  if (await realpath(storeRoot).catch(() => '') !== storeRoot) throw new Error('Record capture/2 store path is aliased');
  return storeRoot;
}

async function verifyCaptureExecutable(path: string): Promise<void> {
  if (!isAbsolute(path) || resolve(path) !== path) throw new Error('APPARENT_ENGINE_CAPTURE_EXECUTABLE must be a normalized absolute path');
  const parts = path.split(sep).filter(Boolean);
  let current = parse(path).root;
  for (const part of parts) {
    current = join(current, part);
    const info = await lstat(current);
    if (info.isSymbolicLink()) throw new Error('Capture/2 Engine executable path cannot contain symlinks');
  }
  const info = await lstat(path);
  if (!info.isFile() || (info.mode & 0o111) === 0 || await realpath(path).catch(() => '') !== path)
    throw new Error('Capture/2 Engine executable must be an executable regular file');
}

async function verifyCaptureRuntime(path: string): Promise<void> {
  if (!isAbsolute(path) || resolve(path) !== path) throw new Error('APPARENT_ENGINE_RUNTIME must be a normalized absolute path');
  const parts = path.split(sep).filter(Boolean);
  let current = parse(path).root;
  for (const part of parts) {
    current = join(current, part);
    const info = await lstat(current);
    if (info.isSymbolicLink()) throw new Error('APPARENT_ENGINE_RUNTIME cannot contain symlinks');
  }
  const info = await lstat(path);
  if (!info.isDirectory() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0 || await realpath(path).catch(() => '') !== path)
    throw new Error('APPARENT_ENGINE_RUNTIME must be a private owner-only directory');
}

async function defaultHostFactory(paths: Parameters<Capture2HostFactory>[0]) {
  const storeRoot = await ensurePrivateStoreRoot(paths.controlRoot);
  await verifyCaptureRuntime(paths.runtime);
  await verifyCaptureExecutable(paths.executable);
  const [{ AbsentDestinationAllocator, SqliteCaptureMappingStore }, { createCapture2Host }] = await Promise.all([
    import('@apparent/record/host'), import('@apparent/record/host/capture2'),
  ]);
  const client = new CaptureClient(launchCaptureEngine(paths.executable, paths.runtime, 40_000));
  try {
    const host = createCapture2Host({ client, mappings: new SqliteCaptureMappingStore(storeRoot), destinations: new AbsentDestinationAllocator(storeRoot, paths.destinationRoot) });
    return { host, close: () => client.close() };
  } catch (error) { await client.close(); throw error; }
}

async function loadPublicIdentity(controlRoot: string, requestId: string): Promise<string> {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(requestId)) throw new Error('Expected an Agent capture/2 request ID');
  if (await realpath(controlRoot).catch(() => '') !== controlRoot) throw new Error('Agent capture/2 control directory is unavailable');
  const directory = resolve(controlRoot, 'capture2-requests-v1');
  for (const path of [controlRoot, directory]) {
    const info = await lstat(path).catch(() => undefined);
    if (!info?.isDirectory() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw new Error('Agent capture/2 control directory is unavailable');
  }
  const path = resolve(directory, `${requestId}.json`);
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
    .catch(() => { throw new Error('No retained Agent capture/2 request has this ID'); });
  try {
    const before = await file.stat({ bigint: true });
    if (!before.isFile() || before.size > 8192n || before.nlink !== 1n || before.uid !== BigInt(process.getuid?.() ?? -1) || (before.mode & 0o077n) !== 0n) throw new Error('Invalid Agent capture/2 identity metadata');
    const buffer = Buffer.alloc(8193);
    let bytesRead = 0;
    while (bytesRead < buffer.length) {
      const result = await file.read(buffer, bytesRead, buffer.length - bytesRead, bytesRead);
      if (result.bytesRead === 0) break;
      bytesRead += result.bytesRead;
    }
    if (bytesRead > 8192) throw new Error('Invalid Agent capture/2 identity metadata');
    const after = await file.stat({ bigint: true });
    const named = await lstat(path, { bigint: true }).catch(() => undefined);
    if (!named?.isFile() || named.isSymbolicLink() || named.dev !== before.dev || named.ino !== before.ino || named.size !== before.size || named.mode !== before.mode || named.uid !== before.uid || named.nlink !== before.nlink || named.mtimeNs !== before.mtimeNs || named.ctimeNs !== before.ctimeNs ||
        before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.size !== BigInt(bytesRead) ||
        before.mode !== after.mode || before.uid !== after.uid || before.nlink !== after.nlink || before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs)
      throw new Error('Agent capture/2 identity metadata changed while reading');
    const entry = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, bytesRead)));
    if (Object.keys(entry).some(key => !['schema', 'requestId', 'destinationPath', 'createdAtMs'].includes(key)) || entry.schema !== 'apparent.agent-capture2-request/1' || entry.requestId !== requestId || !Number.isSafeInteger(entry.createdAtMs) || typeof entry.destinationPath !== 'string' || !isAbsolute(entry.destinationPath) || resolve(entry.destinationPath) !== entry.destinationPath || !entry.destinationPath.endsWith('.apparent')) throw new Error('Invalid Agent capture/2 identity metadata');
    return entry.destinationPath as string;
  } finally { await file.close(); }
}

export async function retainPublicIdentity(controlRoot: string, requestId: string, destinationPath: string): Promise<void> {
  const directory = resolve(controlRoot, 'capture2-requests-v1');
  await mkdir(directory, { recursive: true, mode: 0o700 });
  for (const path of [controlRoot, directory]) {
    const info = await lstat(path);
    if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0)
      throw new Error('Agent capture/2 control directory must be private and owned by the current user');
  }
  const path = resolve(directory, `${requestId}.json`);
  const file = await open(path, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
  try {
    await file.writeFile(JSON.stringify({ schema: 'apparent.agent-capture2-request/1', requestId, destinationPath, createdAtMs: Date.now() }));
    await file.sync();
  } finally { await file.close(); }
  const parent = await open(directory, constants.O_RDONLY);
  try { await parent.sync(); } finally { await parent.close(); }
}

export function parseCapture2ControlArgs(args: string[]): { method: 'inspect' | 'stop' | 'cancel'; requestId: string } {
  if (args.length !== 4 || args[0] !== '--protocol' || args[1] !== 'capture2' || args.slice(2).includes('--protocol'))
    throw new Error('capture requires one --protocol capture2 followed by inspect, stop or cancel and an Agent capture/2 request ID');
  const method = args[2];
  const requestId = args[3];
  if (!['inspect', 'stop', 'cancel'].includes(method)) throw new Error('capture --protocol capture2 requires inspect, stop or cancel and an Agent capture/2 request ID');
  return { method: method as 'inspect' | 'stop' | 'cancel', requestId };
}

/** Capture/2 stays explicit; its admitted production provider is Linux X11 only. */
export function parseCapture2RecordArgs(args: string[]): { durationSeconds: number; out: string; sourceId: string } {
  let duration: number | undefined, out: string | undefined, sourceId: string | undefined, protocol = false;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--protocol' && !protocol && args[i + 1] === 'capture2') { protocol = true; i++; continue; }
    if (arg === '--duration' && duration === undefined) duration = Number(args[++i]);
    else if (arg === '--out' && out === undefined) out = args[++i];
    else if (arg === '--source' && sourceId === undefined) sourceId = args[++i];
    else throw new Error(`Unknown, duplicate, or invalid capture/2 record option: ${arg}`);
  }
  if (!protocol || !Number.isFinite(duration) || duration! <= 0 || duration! > 3600 || !out || !sourceId)
    throw new Error('record --protocol capture2 requires --duration (greater than 0 and at most 3600 seconds), --out, and an exact discovered --source ID');
  if (!isAbsolute(out!) || resolve(out!) !== out || !out!.endsWith('.apparent') || out!.endsWith('/.apparent'))
    throw new Error('--out must be a normalized absolute .apparent destination');
  if (!sourceId || sourceId.startsWith('-') || sourceId.length > 4096) throw new Error('--source requires an exact discovered source ID');
  return { durationSeconds: duration!, out: out!, sourceId: sourceId! };
}

export async function recordWithCapture2(
  args: string[],
  config: { executable?: string; runtime?: string; controlRoot?: string; factory?: Capture2HostFactory; clock?: Capture2Clock; interrupted?: () => boolean; persistIdentity?: typeof retainPublicIdentity } = {},
): Promise<number> {
  const parsed = parseCapture2RecordArgs(args);
  const executable = config.executable ?? process.env.APPARENT_ENGINE_CAPTURE_EXECUTABLE;
  const runtime = config.runtime ?? process.env.APPARENT_ENGINE_RUNTIME;
  const controlRoot = config.controlRoot ?? process.env.APPARENT_AGENT_CONTROL_ROOT ?? resolve(homedir(), '.local/share/apparent/agent');
  const destinationRoot = resolve(parsed.out, '..');
  if (!executable || !runtime || !isAbsolute(executable) || resolve(executable) !== executable || !isAbsolute(runtime) || resolve(runtime) !== runtime)
    throw new Error('Capture/2 requires APPARENT_ENGINE_CAPTURE_EXECUTABLE and APPARENT_ENGINE_RUNTIME as normalized absolute paths; no protocol fallback is attempted');
  if (!isAbsolute(controlRoot) || resolve(controlRoot) !== controlRoot || controlRoot === parsed.out || parsed.out.startsWith(`${controlRoot}/`) || controlRoot.startsWith(`${parsed.out}/`))
    throw new Error('Agent capture/2 control root must be normalized, absolute, and separate from the output destination');
  await mkdir(controlRoot, { recursive: true, mode: 0o700 });
  const rootInfo = await lstat(controlRoot);
  if (!rootInfo.isDirectory() || rootInfo.isSymbolicLink() || rootInfo.uid !== process.getuid?.() || (rootInfo.mode & 0o077) !== 0 || await realpath(controlRoot).catch(() => '') !== controlRoot)
    throw new Error('Agent capture/2 control root must be private and owned by the current user');
  const composed = await (config.factory ?? defaultHostFactory)({ controlRoot, destinationRoot, runtime, executable });
  const clock = config.clock ?? systemClock;
  let signalRequested = false;
  const signal = () => { signalRequested = true; };
  const interrupted = () => signalRequested || config.interrupted?.() === true;
  process.on('SIGINT', signal); process.on('SIGTERM', signal);
  let confirmedClose = false;
  let finalPhase: Capture2Receipt['phase'] = 'unknown';
  let requestId: string | undefined;
  try {
    const ready = await composed.host.readiness();
    if (!ready.ready) throw new Error(`Capture/2 provider unavailable: ${ready.reason ?? 'not ready'}`);
    const discovery = await composed.host.discoverSources();
    const matches = discovery.sources.filter(source => source.id === parsed.sourceId);
    if (discovery.state !== 'available' || matches.length !== 1) throw new Error(matches.length ? 'Discovered source ID is ambiguous' : 'Source ID was not present in current capture/2 discovery');
    if (interrupted()) throw new Error('Interrupted before capture start; no request was submitted');
    requestId = randomUUID();
    await (config.persistIdentity ?? retainPublicIdentity)(controlRoot, requestId, parsed.out);
    if (interrupted()) throw new Error(`Interrupted after retaining capture/2 request ${requestId}; no start was dispatched`);
    process.stderr.write(`Capture/2 request ID: ${requestId}\n`);
    let receipt = await composed.host.start({ requestId, source: matches[0], options: { audio: 'none', localOnly: true }, invocation: { invocationId: randomBytes(16).toString('hex'), observedAtMs: Date.now(), kind: 'explicit' }, destinationPath: parsed.out });
    const startupBy = clock.now() + 30_000;
    while (receipt.phase === 'starting' && clock.now() < startupBy && !interrupted()) {
      await clock.sleep(200); receipt = await composed.host.inspect(requestId);
    }
    if (receipt.phase === 'starting' && !interrupted()) receipt = await composed.host.cancel(requestId);
    if (receipt.phase === 'recording') {
      const stopAt = clock.now() + parsed.durationSeconds * 1000;
      while (clock.now() < stopAt && !interrupted()) {
        await clock.sleep(Math.min(250, Math.max(1, stopAt - clock.now())));
        receipt = await composed.host.inspect(requestId);
        if (receipt.phase !== 'recording') break;
      }
      if (receipt.phase === 'recording') receipt = await composed.host.stop(requestId);
    } else if (interrupted() && ['starting', 'submitted-unknown', 'unknown'].includes(receipt.phase)) {
      receipt = await composed.host.cancel(requestId);
    }
    const finishBy = clock.now() + 60_000;
    while (!['failed', 'cancelled', 'unknown'].includes(receipt.phase) && clock.now() < finishBy) {
      if (receipt.phase === 'materials-ready' || receipt.phase === 'processing') break;
      await clock.sleep(200); receipt = await composed.host.inspect(requestId);
    }
    finalPhase = receipt.phase;
  } finally {
    process.removeListener('SIGINT', signal); process.removeListener('SIGTERM', signal);
    composed.host.dispose();
    const closed = await composed.close();
    confirmedClose = ['already-exited', 'eof-exited', 'exit-observed'].includes(closed);
    if (!['already-exited', 'eof-exited', 'exit-observed'].includes(closed)) process.stderr.write(`Capture/2 transport close is uncertain (${closed}); retain the request identity for reconciliation.\n`);
  }
  const outcome = !confirmedClose ? 'close-unverified' : finalPhase === 'failed' ? 'failed' : finalPhase === 'cancelled' ? 'cancelled' : finalPhase === 'unknown' ? 'uncertain' : finalPhase === 'processing' || finalPhase === 'materials-ready' ? 'pending-saved-capability-unavailable' : 'pending';
  process.stdout.write(`${JSON.stringify({ ...(requestId ? { requestId } : {}), state: finalPhase, outcome })}\n`);
  return ['failed', 'cancelled'].includes(finalPhase) ? 1 : 3;
}

export function parseCapture2SourcesArgs(args: string[]): { json: boolean } {
  if (args.length > 1 || (args.length === 1 && args[0] !== '--json')) throw new Error('capture --protocol capture2 sources accepts only optional --json');
  return { json: args[0] === '--json' };
}

/** Read-only source discovery through the published SDK; it never composes Record or submits capture authority. */
export async function discoverCapture2Sources(args: string[], config: { executable?: string; runtime?: string; transportFactory?: (executable: string, runtime: string) => CaptureTransport } = {}): Promise<number> {
  const { json } = parseCapture2SourcesArgs(args);
  const executable = config.executable ?? process.env.APPARENT_ENGINE_CAPTURE_EXECUTABLE;
  const runtime = config.runtime ?? process.env.APPARENT_ENGINE_RUNTIME;
  if (!executable || !runtime || !isAbsolute(executable) || resolve(executable) !== executable || !isAbsolute(runtime) || resolve(runtime) !== runtime)
    throw new Error('Capture/2 source discovery requires APPARENT_ENGINE_CAPTURE_EXECUTABLE and APPARENT_ENGINE_RUNTIME as normalized absolute paths; no protocol fallback is attempted');
  await verifyCaptureRuntime(runtime);
  await verifyCaptureExecutable(executable);
  const client = new CaptureClient((config.transportFactory ?? ((bin, dir) => launchCaptureEngine(bin, dir, 40_000)))(executable, runtime));
  let discovery;
  let handshake;
  let close = 'uncertain';
  try {
    handshake = await client.handshake(5000);
    if (handshake.readiness !== 'ready' || !handshake.capabilities.sourceDiscovery) {
      discovery = { state: 'unavailable', sources: [], reason: handshake.readiness !== 'ready' ? 'Capture provider is not ready' : 'Source discovery is unavailable' } as const;
    } else discovery = await client.discoverSources(5000);
  } finally { close = await client.close(); }
  const confirmed = ['already-exited', 'eof-exited', 'exit-observed'].includes(close);
  if (json) process.stdout.write(`${JSON.stringify({ state: discovery!.state, sources: discovery!.sources, ...(discovery!.reason ? { reason: discovery!.reason } : {}), transportClose: confirmed ? close : 'uncertain' })}\n`);
  else {
    if (discovery!.state === 'available' && discovery!.sources.length) {
      for (const source of discovery!.sources) process.stdout.write(`${source.id}  ${source.kind}  ${source.bounds.width}×${source.bounds.height} at (${source.bounds.x}, ${source.bounds.y})${source.bounds.scale === undefined ? '' : ` scale ${source.bounds.scale}`}\n`);
    } else process.stdout.write(`No sources available${discovery!.reason ? `: ${discovery!.reason}` : ''}\n`);
    if (!confirmed) process.stderr.write(`Capture/2 transport close is uncertain (${close}).\n`);
  }
  return discovery!.state === 'available' && confirmed ? 0 : 3;
}

export async function controlCapture2(args: string[], config: { executable?: string; runtime?: string; controlRoot?: string; factory?: Capture2HostFactory } = {}): Promise<number> {
  const { method, requestId } = parseCapture2ControlArgs(args);
  const executable = config.executable ?? process.env.APPARENT_ENGINE_CAPTURE_EXECUTABLE;
  const runtime = config.runtime ?? process.env.APPARENT_ENGINE_RUNTIME;
  const controlRoot = config.controlRoot ?? process.env.APPARENT_AGENT_CONTROL_ROOT ?? resolve(homedir(), '.local/share/apparent/agent');
  if (!executable || !runtime || !isAbsolute(executable) || resolve(executable) !== executable || !isAbsolute(runtime) || resolve(runtime) !== runtime || !isAbsolute(controlRoot) || resolve(controlRoot) !== controlRoot)
    throw new Error('Capture/2 control requires explicit normalized absolute provider, runtime, and control-root paths');
  const destinationPath = await loadPublicIdentity(controlRoot, requestId);
  const info = await lstat(controlRoot);
  if (info.isSymbolicLink() || !info.isDirectory() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0 || await realpath(controlRoot).catch(() => '') !== controlRoot)
    throw new Error('Agent capture/2 control root must be private and owned by the current user');
  const composed = await (config.factory ?? defaultHostFactory)({ controlRoot, destinationRoot: resolve(destinationPath, '..'), runtime, executable });
  let receipt: Capture2Receipt;
  let closeVerified = false;
  try {
    receipt = await composed.host[method](requestId);
  } finally {
    composed.host.dispose();
    const closed = await composed.close();
    closeVerified = ['already-exited', 'eof-exited', 'exit-observed'].includes(closed);
    if (!closeVerified) process.stderr.write(`Capture/2 transport close is uncertain (${closed}); retain the request identity for reconciliation.\n`);
  }
  const outcome = !closeVerified ? 'close-unverified' : receipt.phase === 'unknown' ? 'uncertain' : receipt.phase === 'failed' ? 'failed' : receipt.phase === 'cancelled' ? 'cancelled' : receipt.phase === 'processing' || receipt.phase === 'materials-ready' ? 'pending-saved-capability-unavailable' : 'pending';
  process.stdout.write(`${JSON.stringify({ requestId, state: receipt.phase, outcome })}\n`);
  return ['failed', 'cancelled'].includes(receipt.phase) ? (receipt.phase === 'cancelled' && method === 'cancel' && closeVerified ? 0 : 1) : 3;
}

import { randomBytes, randomUUID } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, mkdir, open, realpath } from 'node:fs/promises';
import { isAbsolute, join, parse, resolve, sep } from 'node:path';
import { homedir } from 'node:os';
import { performance } from 'node:perf_hooks';
import { CaptureClient, launchCaptureEngine, preflightCapturePublication, type CaptureTransport } from '@apparent/engine-sdk/capture';
import { Media2Client } from '@apparent/engine-sdk/media2';
import { launchCaptureRecoveryPeer, launchMedia2RecoveryPeer, type RecoveryPeerRelease } from '@apparent/engine-sdk/recovery';
import { DEFAULT_RECORD_DURATION_SECONDS } from '../recording.js';
import type { Capture2Host, Capture2Receipt } from '@apparent/record/host/capture2';
import type { RecordPublicationOutput } from '@apparent/record/publication';
import type { RecordCaptureCoordinator } from '@apparent/record/host/capture2-coordinator';

export type Capture2Clock = { now(): number; sleep(ms: number): Promise<void> };
const systemClock: Capture2Clock = { now: () => performance.now(), sleep: ms => new Promise(resolveSleep => setTimeout(resolveSleep, ms)) };
export type Capture2PublicOutput = Pick<RecordPublicationOutput, 'captureFacts' | 'resolveHandoff'>;
export type Capture2HostFactory = (paths: { controlRoot: string; destinationRoot: string; runtime: string; executable: string }) => Promise<{
  host: Capture2Host;
  allocateDestination(requestId: string, exactPath?: string): Promise<string>;
  close(): Promise<string>;
  publicationOutput?: Capture2PublicOutput;
}>;

export type RecoveryCoordinatorFactory = (paths: { controlRoot: string; destinationRoot: string; runtime: string; captureExecutable: string; mediaExecutable: string }) => Promise<{
  coordinator: RecordCaptureCoordinator;
  release(): Promise<[RecoveryPeerRelease, RecoveryPeerRelease]>;
}>;

async function defaultRecoveryCoordinatorFactory(paths: Parameters<RecoveryCoordinatorFactory>[0]) {
  const storeRoot = await ensurePrivateStoreRoot(paths.controlRoot);
  await verifyCaptureRuntime(paths.runtime);
  await verifyCaptureExecutable(paths.captureExecutable);
  await verifyCaptureExecutable(paths.mediaExecutable);
  const [{ createRecordCaptureCoordinator }, { SqliteCaptureMappingStore, AbsentDestinationAllocator }] = await Promise.all([
    import('@apparent/record/host/capture2-coordinator'), import('@apparent/record/host'),
  ]);
  const capturePeer = await launchCaptureRecoveryPeer(paths.captureExecutable, paths.runtime);
  let mediaPeer: Awaited<ReturnType<typeof launchMedia2RecoveryPeer>> | undefined;
  let injected = false;
  try {
    mediaPeer = await launchMedia2RecoveryPeer(paths.mediaExecutable, paths.runtime);
    // Validate the ordinary SDK client contracts while both recovery peers
    // are still caller-owned and have not been handed to Record.
    await capturePeer.client.handshake();
    await mediaPeer.client.handshake();
    // Passing either client to Record transfers use to a borrower. If its
    // asynchronous construction rejects, no public coordinator handle exists
    // from which this caller can prove a drain, so these peers stay owned.
    injected = true;
    const coordinator = await createRecordCaptureCoordinator({
      captureClient: capturePeer.client, mediaClient: mediaPeer.client, privateControlRoot: storeRoot,
      mappings: new SqliteCaptureMappingStore(storeRoot),
      destinations: new AbsentDestinationAllocator(storeRoot, paths.destinationRoot),
      createStartIntent() { throw new Error('Recovery cannot start a fresh capture'); },
      trustedMaterialsRoot() { throw new Error('Recovery cannot admit a new retained-materials root'); },
      selectProcessing() { throw new Error('Recovery cannot make a new processing selection'); },
      preflight: preflightCapturePublication,
    });
    return { coordinator, async release() {
      const settled = await Promise.allSettled([capturePeer.release(), mediaPeer!.release()]);
      if (settled.some(result => result.status === 'rejected')) throw new AggregateError(settled.filter(result => result.status === 'rejected').map(result => result.reason), 'Recovery peer release failed');
      return settled.map(result => (result as PromiseFulfilledResult<RecoveryPeerRelease>).value) as [RecoveryPeerRelease, RecoveryPeerRelease];
    } };
  } catch (error) {
    // A second launch failure leaves the first peer genuinely un-injected.
    if (!injected) await Promise.allSettled([capturePeer.release(), ...(mediaPeer ? [mediaPeer.release()] : [])]);
    throw error;
  }
}

/** Reconcile only durable Record-owned capture/processing/publication identities. */
export async function recoverRecordCaptures(args: string[], config: {
  captureExecutable?: string; mediaExecutable?: string; runtime?: string; controlRoot?: string;
  destinationRoot?: string; factory?: RecoveryCoordinatorFactory;
} = {}): Promise<number> {
  if (args.length) throw new Error('recover takes no operands or options');
  const captureExecutable = config.captureExecutable ?? process.env.APPARENT_ENGINE_CAPTURE_EXECUTABLE;
  const mediaExecutable = config.mediaExecutable ?? process.env.APPARENT_ENGINE_MEDIA2_BIN;
  const runtime = config.runtime ?? process.env.APPARENT_ENGINE_RUNTIME;
  const controlRoot = config.controlRoot ?? process.env.APPARENT_AGENT_CONTROL_ROOT ?? resolve(homedir(), '.local/share/apparent/agent');
  const destinationRoot: string = config.destinationRoot ?? await import('../bundle.js').then(({ resolveCaptures }) => resolveCaptures());
  if (!captureExecutable || !mediaExecutable || !runtime || !isAbsolute(captureExecutable) || resolve(captureExecutable) !== captureExecutable ||
      !isAbsolute(mediaExecutable) || resolve(mediaExecutable) !== mediaExecutable || !isAbsolute(runtime) || resolve(runtime) !== runtime ||
      !isAbsolute(controlRoot) || resolve(controlRoot) !== controlRoot || !isAbsolute(destinationRoot) || resolve(destinationRoot) !== destinationRoot)
    throw new Error('recover requires normalized absolute Capture/2, media/2, runtime, control, and capture-storage paths');
  if (controlRoot === destinationRoot || controlRoot.startsWith(`${destinationRoot}/`) || destinationRoot.startsWith(`${controlRoot}/`))
    throw new Error('Recovery control and capture storage must be separate');
  await mkdir(controlRoot, { recursive: true, mode: 0o700 });
  const rootInfo = await lstat(controlRoot);
  if (!rootInfo.isDirectory() || rootInfo.isSymbolicLink() || rootInfo.uid !== process.getuid?.() || (rootInfo.mode & 0o077) !== 0 || await realpath(controlRoot).catch(() => '') !== controlRoot)
    throw new Error('Agent capture/2 control root must be private and owned by the current user');
  const composed = await (config.factory ?? defaultRecoveryCoordinatorFactory)({ controlRoot, destinationRoot, runtime, captureExecutable, mediaExecutable });
  let reconciliationFailed = false;
  let peerRelease: [RecoveryPeerRelease, RecoveryPeerRelease] | undefined;
  let results: unknown[] = [];
  let issues: readonly unknown[] = [];
  let shutdownReport: Awaited<ReturnType<RecordCaptureCoordinator['shutdown']>> | undefined;
  let disposed = false;
  try {
    try { await composed.coordinator.reconcile(); }
    catch { reconciliationFailed = true; }
    const [facts, observedIssues] = await Promise.all([
      composed.coordinator.captureFacts.readFacts().catch(() => undefined),
      Promise.resolve(composed.coordinator.processingIssues()),
    ]);
    issues = observedIssues;
    if (!facts) reconciliationFailed = true;
    if (facts) {
      for (const fact of facts.captures) {
        let savedBundlePath: string | undefined;
        if (fact.phase === 'saved' && fact.artifactId) {
          try {
            const resolved = await composed.coordinator.resolveHandoff({ captureId: fact.captureId, artifactId: fact.artifactId });
            const bundle = resolved?.handoff?.savedBundle;
            if (resolved?.captureId === fact.captureId && resolved.artifactId === fact.artifactId && bundle?.status === 'saved' &&
                typeof bundle.bundlePath === 'string' && await realpath(bundle.bundlePath).catch(() => '') === bundle.bundlePath) savedBundlePath = bundle.bundlePath;
          } catch { /* a public Saved fact without exact handoff resolution stays pathless */ }
        }
        results.push({ captureId: fact.captureId, state: savedBundlePath ? 'saved' : fact.phase === 'saved' ? 'unknown' : fact.phase,
          ...(savedBundlePath ? { bundlePath: savedBundlePath } : {}) });
      }
    }
    const reportedIds = new Set(results.map((result: any) => result.captureId));
    const ownership = composed.coordinator.runtime.unresolvedOwnership();
    if (ownership.storageUnavailable || ownership.reconciliation !== 'complete') reconciliationFailed = true;
    for (const capture of composed.coordinator.runtime.captures?.() ?? []) {
      if (!('captureId' in capture) || reportedIds.has(capture.captureId)) continue;
      results.push({ captureId: capture.captureId, state: capture.phase === 'ready' ? 'unknown' : capture.phase });
      reportedIds.add(capture.captureId);
    }
    for (const unresolved of ownership.requestIds) {
      if (reportedIds.has(unresolved)) continue;
      results.push({ captureId: unresolved, state: 'unknown' });
      reportedIds.add(unresolved);
    }
  } finally {
    try { shutdownReport = await composed.coordinator.shutdown(); }
    catch { reconciliationFailed = true; }
    if (!shutdownReport || shutdownReport.scope !== 'record-coordinator' || shutdownReport.admission !== 'closed' || shutdownReport.localDrain !== 'complete') {
      reconciliationFailed = true;
    } else {
      try { await composed.coordinator.dispose(); disposed = true; }
      catch { reconciliationFailed = true; }
      if (disposed) {
        peerRelease = await composed.release().catch(() => undefined);
        if (!peerRelease || peerRelease.length !== 2 || peerRelease.some(item => item.scope !== 'client-connection' || item.connection !== 'released' ||
            item.runtimeClearance !== 'not-established')) reconciliationFailed = true;
      }
    }
  }
  const shutdownValid = shutdownReport?.scope === 'record-coordinator' && shutdownReport.admission === 'closed' && shutdownReport.localDrain === 'complete';
  const releaseConfirmed = !!peerRelease && peerRelease.length === 2 && peerRelease.every(item => item.scope === 'client-connection' && item.connection === 'released');
  const closureConfirmed = !!peerRelease && peerRelease.length === 2 && peerRelease.every(item => ['confirmed', 'peer-exit-observed'].includes(item.inputClosure));
  if (!releaseConfirmed || !closureConfirmed) process.stderr.write('Recovery peer connection release or input closure is unconfirmed; no completion is claimed.\n');
  process.stdout.write(`${JSON.stringify({ captures: results, processingIssues: issues, reconciliation: reconciliationFailed ? 'unknown' : 'complete',
    shutdown: shutdownReport ? { scope: shutdownReport.scope, admission: shutdownReport.admission, localDrain: shutdownReport.localDrain,
      runtimeClearance: shutdownReport.runtimeClearance, work: shutdownReport.work } : 'unknown',
    peerRelease: peerRelease ?? 'unknown', peerInputClosure: closureConfirmed ? 'confirmed' : 'unconfirmed' })}\n`);
  if (reconciliationFailed || !shutdownValid || !disposed || !releaseConfirmed || !closureConfirmed || shutdownReport?.runtimeClearance !== 'clear' ||
      shutdownReport.work.length !== 0 || shutdownReport.nextShortcutAdmission.state !== 'unsupported' || issues.length ||
      results.some((result: any) => ['checking', 'starting', 'recording', 'stopping', 'cancelling', 'stop-failed', 'uncertain', 'processing', 'unknown'].includes(result.state))) return 3;
  return results.some((result: any) => ['failed', 'cancelled'].includes(result.state)) ? 1 : 0;
}

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
  const [{ AbsentDestinationAllocator, SqliteCaptureMappingStore }, { createCapture2Host }, { createRecordPublicationOutput }] = await Promise.all([
    import('@apparent/record/host'), import('@apparent/record/host/capture2'), import('@apparent/record/publication'),
  ]);
  const client = new CaptureClient(launchCaptureEngine(paths.executable, paths.runtime, 40_000));
  try {
    const destinations = new AbsentDestinationAllocator(storeRoot, paths.destinationRoot);
    const host = createCapture2Host({ client, mappings: new SqliteCaptureMappingStore(storeRoot), destinations });
    const publicationOutput = createRecordPublicationOutput(client, storeRoot, preflightCapturePublication);
    return { host, publicationOutput, allocateDestination: async (requestId: string, exactPath?: string) => exactPath
      ? (await destinations.reserveExact(requestId, exactPath)).bundlePath
      : (await destinations.allocate(requestId)).bundlePath, close: () => client.close() };
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
export function parseCapture2RecordArgs(args: string[]): { durationSeconds: number; out?: string; sourceId: string } {
  let duration: number | undefined, durationSeen = false, out: string | undefined, sourceId: string | undefined, protocol = false;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--protocol' && !protocol && args[i + 1] === 'capture2') { protocol = true; i++; continue; }
    if (arg === '--duration' && !durationSeen) { durationSeen = true; duration = Number(args[++i]); }
    else if (arg === '--out' && out === undefined) {
      out = args[++i];
      if (!out || out.startsWith('--')) throw new Error('--out requires a normalized absolute .apparent destination');
    }
    else if (arg === '--source' && sourceId === undefined) sourceId = args[++i];
    else throw new Error(`Unknown, duplicate, or invalid capture/2 record option: ${arg}`);
  }
  const durationSeconds = duration ?? DEFAULT_RECORD_DURATION_SECONDS;
  if (!protocol || !Number.isFinite(durationSeconds) || durationSeconds <= 0 || durationSeconds > 3600 || !sourceId)
    throw new Error('record --protocol capture2 requires an exact discovered --source ID and a --duration greater than 0 and at most 3600 seconds when supplied');
  if (out !== undefined && (!isAbsolute(out) || resolve(out) !== out || !out.endsWith('.apparent') || out.endsWith('/.apparent')))
    throw new Error('--out must be a normalized absolute .apparent destination');
  if (!sourceId || sourceId.startsWith('-') || sourceId.length > 4096) throw new Error('--source requires an exact discovered source ID');
  return { durationSeconds, ...(out === undefined ? {} : { out }), sourceId: sourceId! };
}

export async function recordWithCapture2(
  args: string[],
  config: { executable?: string; runtime?: string; controlRoot?: string; factory?: Capture2HostFactory; clock?: Capture2Clock; interrupted?: () => boolean; persistIdentity?: typeof retainPublicIdentity } = {},
): Promise<number> {
  const parsed = parseCapture2RecordArgs(args);
  const executable = config.executable ?? process.env.APPARENT_ENGINE_CAPTURE_EXECUTABLE;
  const runtime = config.runtime ?? process.env.APPARENT_ENGINE_RUNTIME;
  const controlRoot = config.controlRoot ?? process.env.APPARENT_AGENT_CONTROL_ROOT ?? resolve(homedir(), '.local/share/apparent/agent');
  const destinationRoot = parsed.out ? resolve(parsed.out, '..') : await import('../bundle.js').then(({ resolveCaptures }) => resolveCaptures());
  if (!executable || !runtime || !isAbsolute(executable) || resolve(executable) !== executable || !isAbsolute(runtime) || resolve(runtime) !== runtime)
    throw new Error('Capture/2 requires APPARENT_ENGINE_CAPTURE_EXECUTABLE and APPARENT_ENGINE_RUNTIME as normalized absolute paths; no protocol fallback is attempted');
  if (!isAbsolute(controlRoot) || resolve(controlRoot) !== controlRoot || (parsed.out && (controlRoot === parsed.out || parsed.out.startsWith(`${controlRoot}/`) || controlRoot.startsWith(`${parsed.out}/`))))
    throw new Error('Agent capture/2 control root must be normalized, absolute, and separate from the output destination');
  if (!parsed.out && (destinationRoot === controlRoot || destinationRoot.startsWith(`${controlRoot}/`) || controlRoot.startsWith(`${destinationRoot}/`)))
    throw new Error('Configured capture storage must be separate from the Agent control root');
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
    const destinationPath = await composed.allocateDestination(requestId, parsed.out);
    await (config.persistIdentity ?? retainPublicIdentity)(controlRoot, requestId, destinationPath);
    if (interrupted()) throw new Error(`Interrupted after retaining capture/2 request ${requestId}; no start was dispatched`);
    process.stderr.write(`Capture/2 request ID: ${requestId}\n`);
    let receipt = await composed.host.start({ requestId, source: matches[0], options: { audio: 'none', localOnly: true }, invocation: { invocationId: randomBytes(16).toString('hex'), observedAtMs: Date.now(), kind: 'explicit' }, ...(parsed.out ? { destinationPath } : {}) });
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
  const savedBundlePath = method === 'inspect' && closeVerified && composed.publicationOutput
    ? await authenticatedSavedBundlePath(composed.publicationOutput, requestId, destinationPath)
    : undefined;
  const outcome = savedBundlePath !== undefined ? 'saved' : !closeVerified ? 'close-unverified' : receipt.phase === 'unknown' ? 'uncertain' : receipt.phase === 'failed' ? 'failed' : receipt.phase === 'cancelled' ? 'cancelled' : receipt.phase === 'processing' || receipt.phase === 'materials-ready' ? 'pending-saved-capability-unavailable' : 'pending';
  process.stdout.write(`${JSON.stringify({ requestId, state: savedBundlePath === undefined ? receipt.phase : 'saved', outcome, ...(savedBundlePath === undefined ? {} : { bundlePath: savedBundlePath }) })}\n`);
  if (savedBundlePath !== undefined) return 0;
  return ['failed', 'cancelled'].includes(receipt.phase) ? (receipt.phase === 'cancelled' && method === 'cancel' && closeVerified ? 0 : 1) : 3;
}

async function authenticatedSavedBundlePath(output: Capture2PublicOutput, requestId: string, expectedPath: string): Promise<string | undefined> {
  try {
    const facts = await output.captureFacts.readFacts();
    const matches = facts.captures.filter(capture => capture.captureId === requestId);
    if (matches.length !== 1 || matches[0].phase !== 'saved' || typeof matches[0].artifactId !== 'string' || !matches[0].artifactId) return undefined;
    const artifactId = matches[0].artifactId;
    const resolution = await output.resolveHandoff({ captureId: requestId, artifactId });
    const bundle = resolution?.handoff?.savedBundle;
    if (resolution?.captureId !== requestId || resolution.artifactId !== artifactId || bundle?.status !== 'saved' || bundle.bundlePath !== expectedPath) return undefined;
    if (await realpath(bundle.bundlePath).catch(() => '') !== expectedPath) return undefined;
    return expectedPath;
  } catch {
    return undefined;
  }
}

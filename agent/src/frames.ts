import { constants } from 'node:fs';
import { lstat, mkdir, open, realpath } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { basename, dirname, isAbsolute, join, resolve, sep } from 'node:path';
import { readManifest, resolveBundle, safeReference } from './bundle.js';
import { createMediaJobId, type DecodeRequest, type DecodeResult, type MediaClient, type MediaFrameRef } from '@apparent/engine-sdk/media';

export const MAX_FRAMES = 48;
export const MAX_FPS = 240;
export interface FramesOptions { bundle: string; from: string; to: string; fps: number; }
export interface FrameSelection { fromMs: number; toMs: number; fps: number; requestedBundleMs: number[]; }

/** Mirrors parse_clock_ms for valid values, with safe-integer overflow rejection. */
export function parseClockMs(input: string): number {
  const value = input.trim();
  let ms: number;
  if (value.includes(':')) {
    const parts = value.split(':');
    if (parts.length !== 2 || !/^\d+$/.test(parts[0])) throw new Error(`invalid time: ${input}`);
    if (!/^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(parts[1])) throw new Error(`invalid time: ${input}`);
    const secs = Number(parts[1]);
    if (!Number.isFinite(secs) || secs < 0) throw new Error(`invalid time: ${input}`);
    ms = Number(parts[0]) * 60_000 + Math.round(secs * 1_000);
  } else if (value.endsWith('ms')) {
    const digits = value.slice(0, -2).trim();
    if (!/^\d+$/.test(digits)) throw new Error(`invalid time: ${input}`);
    ms = Number(digits);
  } else {
    const text = value.endsWith('s') ? value.slice(0, -1) : value;
    if (!/^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(text)) throw new Error(`invalid time: ${input}`);
    const seconds = Number(text);
    if (!Number.isFinite(seconds) || seconds < 0) throw new Error(`invalid time: ${input}`);
    ms = Math.round(seconds * 1_000);
  }
  if (!Number.isSafeInteger(ms) || ms < 0) throw new Error(`invalid time: ${input}`);
  return ms;
}

export function selectFrameTimes(from: string, to: string, fps: number): FrameSelection {
  const fromMs = parseClockMs(from), toMs = parseClockMs(to);
  if (fromMs > toMs) throw new Error('Frame range ends before it starts');
  if (!Number.isSafeInteger(fps) || fps < 1 || fps > MAX_FPS) throw new Error('FPS must be an integer between 1 and 240');
  // Match legacy range semantics: sample [from,to), at the selected frame rate.
  const count = fromMs === toMs ? 1 : Math.ceil((toMs - fromMs) * fps / 1_000);
  if (count > MAX_FRAMES) throw new Error(`Frame request exceeds the ${MAX_FRAMES}-frame limit`);
  return { fromMs, toMs, fps, requestedBundleMs: Array.from({ length: count }, (_, i) => fromMs + Math.floor(i * 1_000 / fps)) };
}

/** Select retained manifest keyframes without decoding or claiming video timestamps. */
export async function savedKeyframes(bundleInput: string, selection: FrameSelection) {
  const bundlePath = await resolveBundle(bundleInput);
  await assertNoSymlinkPath(bundlePath);
  const pending = await lstat(join(bundlePath, '.media-pending')).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  if (pending) throw new Error('Bundle media is still pending (.media-pending)');
  await isEvidenceOnlySnapshot(bundlePath);
  const { value } = await readManifest(bundlePath);
  const margin = Math.floor(Math.ceil(1_000 / selection.fps) / 2);
  const lower = Math.max(0, selection.fromMs - margin), upper = selection.toMs + margin;
  const chosen = new Map<string, { timestampMs: number; path: string }>();
  for (const frame of value.keyframes) {
    if (!Number.isSafeInteger(frame.t) || frame.t < lower || frame.t > upper || typeof frame.file !== 'string') continue;
    if (!safeReference(frame.file) || !frame.file.startsWith('frames/') || !frame.file.endsWith('.png')) continue;
    const path = join(bundlePath, frame.file);
    await assertNoSymlinkPath(path);
    const canonical = await realpath(path).catch(() => undefined);
    if (canonical !== path || !canonical.startsWith(`${join(bundlePath, 'frames')}${sep}`)) continue;
    const prior = chosen.get(canonical);
    if (prior) { prior.timestampMs = Math.min(prior.timestampMs, frame.t); continue; }
    const expected = await lstat(path, {bigint:true}).catch(() => undefined);
    if (!expected?.isFile()) continue;
    const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW).catch(() => undefined);
    if (!file) continue;
    try {
      const stat = await file.stat({bigint:true});
      if (!stat.isFile() || stat.dev !== expected.dev || stat.ino !== expected.ino || stat.size < 24n || stat.size > 16n * 1024n * 1024n) continue;
      await assertNoSymlinkPath(path);
      if (await realpath(path) !== canonical) throw new Error('Saved keyframe path changed before reading');
      const current = await lstat(path, {bigint:true});
      if (current.dev !== stat.dev || current.ino !== stat.ino) throw new Error('Saved keyframe identity changed before reading');
      const header = Buffer.alloc(24);
      let bytesRead = 0;
      while (bytesRead < header.length) {
        const read = await file.read(header, bytesRead, header.length - bytesRead, bytesRead);
        if (read.bytesRead === 0) break;
        bytesRead += read.bytesRead;
      }
      const width = header.readUInt32BE(16), height = header.readUInt32BE(20);
      if (bytesRead !== 24 || header.toString('hex', 0, 8) !== '89504e470d0a1a0a' || !width || !height || width * height > 64_000_000) continue;
      await assertNoSymlinkPath(path);
      const after = await lstat(path, {bigint:true});
      const descriptor = await file.stat({bigint:true});
      if (await realpath(path) !== canonical || after.dev !== stat.dev || after.ino !== stat.ino || descriptor.size !== stat.size || descriptor.mtimeNs !== stat.mtimeNs)
        throw new Error('Saved keyframe identity changed while reading');
      chosen.set(canonical, { timestampMs: frame.t, path: canonical });
      if (chosen.size > MAX_FRAMES) throw new Error(`Saved keyframe selection exceeds the ${MAX_FRAMES}-frame limit`);
    } finally { await file.close(); }
  }
  const frames = [...chosen.values()].sort((a, b) => a.timestampMs - b.timestampMs || a.path.localeCompare(b.path));
  return { bundlePath, frames };
}

export async function hasSourceVideo(bundleInput: string): Promise<boolean> {
  const bundlePath = await resolveBundle(bundleInput);
  await assertNoSymlinkPath(bundlePath);
  const pending = await lstat(join(bundlePath, '.media-pending')).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  if (pending) throw new Error('Bundle media is still pending (.media-pending)');
  if (await isEvidenceOnlySnapshot(bundlePath)) return false;
  const { value } = await readManifest(bundlePath);
  const reference = value.video ?? (await existsRegular(join(bundlePath, 'recording.mp4')) ? 'recording.mp4' : 'recording.mov');
  if (!safeReference(reference)) throw new Error('Bundle has unsafe video metadata');
  const path = join(bundlePath, reference);
  await assertNoSymlinkPath(path);
  const info = await lstat(path).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  if (!info) return false;
  if (!info.isFile()) throw new Error('Bundle source video is not a regular no-follow file');
  return true;
}

/** Validate the producer receipt without granting it authority beyond local evidence-only status. */
async function isEvidenceOnlySnapshot(bundlePath: string): Promise<boolean> {
  const path = join(bundlePath, '.evidence-only.json');
  const expected = await lstat(path, { bigint: true }).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  if (!expected) return false;
  if (!expected.isFile() || expected.isSymbolicLink()) throw new Error('Evidence-only receipt is not a regular no-follow file');
  const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW).catch(() => undefined);
  if (!file) throw new Error('Evidence-only receipt could not be opened safely');
  try {
    const stat = await file.stat({ bigint: true });
    if (!stat.isFile() || stat.dev !== expected.dev || stat.ino !== expected.ino || stat.size <= 0n || stat.size > 16_384n)
      throw new Error('Evidence-only receipt has an unsupported size or identity');
    const bytes = Buffer.alloc(Number(stat.size));
    let offset = 0;
    while (offset < bytes.length) {
      const read = await file.read(bytes, offset, bytes.length - offset, offset);
      if (!read.bytesRead) throw new Error('Evidence-only receipt ended while being read');
      offset += read.bytesRead;
    }
    const after = await file.stat({ bigint: true });
    if (after.dev !== stat.dev || after.ino !== stat.ino || after.size !== stat.size || after.mtimeNs !== stat.mtimeNs)
      throw new Error('Evidence-only receipt changed while being read');
    let receipt: any;
    try { receipt = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); } catch { throw new Error('Evidence-only receipt is invalid JSON'); }
    const keys = ['version', 'captureId', 'server', 'media', 'guidance'];
    if (!receipt || typeof receipt !== 'object' || Array.isArray(receipt) || Object.keys(receipt).sort().join(',') !== keys.sort().join(',') ||
        receipt.version !== 1 || typeof receipt.captureId !== 'string' || !/^[0-9a-f]{32}$/.test(receipt.captureId) ||
        typeof receipt.server !== 'string' || !/^(?:https:\/\/|http:\/\/(?:localhost|127\.0\.0\.1)(?::[0-9]+)?(?:\/|$))/.test(receipt.server) || receipt.media !== 'not downloaded' ||
        receipt.guidance !== 'Use apparent fetch <id> --media --out <new-directory> for the complete recording')
      throw new Error('Evidence-only receipt does not match the published snapshot format');
    return true;
  } finally { await file.close(); }
}

async function assertNoSymlinkPath(path: string, includeLeaf = true): Promise<void> {
  const absolute = resolve(path), parts = absolute.split(sep).filter(Boolean);
  let current: string = sep;
  for (const [index, part] of parts.entries()) {
    current = join(current, part);
    if (!includeLeaf && index === parts.length - 1) break;
    const info = await lstat(current).catch((error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT') return null;
      throw error;
    });
    if (!info) break;
    if (info.isSymbolicLink()) throw new Error('Media path contains a symlink');
  }
}

async function sourceFor(bundleInput: string) {
  const bundlePath = await resolveBundle(bundleInput);
  await assertNoSymlinkPath(bundlePath);
  const marker = await lstat(join(bundlePath, '.media-pending')).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  if (marker) throw new Error('Bundle media is still pending (.media-pending)');
  if (await isEvidenceOnlySnapshot(bundlePath)) throw new Error('Evidence-only bundle cannot be decoded; use retained keyframes');
  const { value } = await readManifest(bundlePath);
  const videoRef = value.video ?? (await existsRegular(join(bundlePath, 'recording.mp4')) ? 'recording.mp4' : 'recording.mov');
  if (!safeReference(videoRef)) throw new Error('Bundle has unsafe video metadata');
  const videoPath = join(bundlePath, videoRef);
  await assertNoSymlinkPath(videoPath);
  const info = await lstat(videoPath).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  if (!info) throw new Error('Bundle source video is missing (possibly removed by retention)');
  if (info.isSymbolicLink() || !info.isFile()) throw new Error('Bundle source video is not a regular no-follow file');
  const file = await open(videoPath, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
  let identity;
  try {
    const stat = await file.stat({ bigint: true });
    if (!stat.isFile() || stat.size > BigInt(Number.MAX_SAFE_INTEGER) || stat.size <= 0n || stat.mtimeNs < 0n)
      throw new Error('Bundle source video identity is unsupported or empty');
    identity = { device: stat.dev.toString(), inode: stat.ino.toString(), size: Number(stat.size), modifiedNs: stat.mtimeNs.toString() };
  } finally { await file.close(); }
  return { bundlePath, videoPath, identity, source: { bundleId: value.id, bundlePath, videoPath, videoOffsetMs: value.videoOffsetMs ?? 0, identity } };
}

export async function verifyFramesExecutable(path: string) {
  if (!isAbsolute(path) || resolve(path) !== path) throw new Error('APPARENT_ENGINE_FRAMES_EXECUTABLE must be a canonical absolute path');
  await assertNoSymlinkPath(path);
  const info = await lstat(path);
  if (!info.isFile() || (info.mode & 0o111) === 0) throw new Error('Frames Engine executable must be an executable regular file');
}

export async function verifyPrivateRuntime(path: string) {
  if (!isAbsolute(path) || resolve(path) !== path) throw new Error('APPARENT_ENGINE_RUNTIME must be a canonical absolute path');
  await assertNoSymlinkPath(path);
  const info = await lstat(path);
  if (!info.isDirectory() || (typeof process.getuid === 'function' && info.uid !== process.getuid()) || (info.mode & 0o077) !== 0)
    throw new Error('APPARENT_ENGINE_RUNTIME must be an owner-only private directory');
}

async function existsRegular(path: string) {
  const info = await lstat(path).catch((e: NodeJS.ErrnoException) => e.code === 'ENOENT' ? null : Promise.reject(e));
  return !!info && info.isFile() && !info.isSymbolicLink();
}

async function freshJobOutput(runtimePath: string, jobId: string) {
  if (!isAbsolute(runtimePath) || resolve(runtimePath) !== runtimePath) throw new Error('APPARENT_ENGINE_RUNTIME must be a canonical absolute path');
  await assertNoSymlinkPath(runtimePath);
  const runtimeStat = await lstat(runtimePath);
  if (!runtimeStat.isDirectory() || (typeof process.getuid === 'function' && runtimeStat.uid !== process.getuid()) || (runtimeStat.mode & 0o077) !== 0)
    throw new Error('APPARENT_ENGINE_RUNTIME must be an owner-only private directory');
  const root = join(runtimePath, 'agent-media');
  await mkdir(root, { recursive: true, mode: 0o700 });
  await assertNoSymlinkPath(root);
  const rootStat = await lstat(root);
  if (!rootStat.isDirectory() || (typeof process.getuid === 'function' && rootStat.uid !== process.getuid()) || (rootStat.mode & 0o077) !== 0)
    throw new Error('Agent media output root is not a private owner directory');
  const jobDirectory = join(root, jobId);
  await mkdir(jobDirectory, { mode: 0o700 });
  const outputDirectory = join(jobDirectory, 'output');
  await mkdir(outputDirectory, { mode: 0o700 });
  return outputDirectory;
}

function validateInventory(frames: MediaFrameRef[], request: DecodeRequest): MediaFrameRef[] {
  if (frames.length !== request.requestedBundleMs.length) throw new Error('Engine returned incomplete frame inventory');
  return frames.map((frame, index) => {
    // MediaClient owns clock/clamp validation. Keep Agent's slot-order check
    // independent of provider seek/clamp policy as that contract evolves.
    if (frame.requestIndex !== index + 1 || frame.requestedBundleMs !== request.requestedBundleMs[index])
      throw new Error('Engine frame inventory does not match the requested slots');
    return frame;
  });
}

async function verifyFrameFiles(frames: MediaFrameRef[], outputDirectory: string) {
  await assertNoSymlinkPath(outputDirectory);
  let totalBytes = 0;
  const maxBytes = 64_000_000 * 4 + 1024 * 1024;
  for (const frame of frames) {
    if (dirname(frame.path) !== outputDirectory || basename(frame.path) !== `frame-${String(frame.requestIndex).padStart(3, '0')}.png`)
      throw new Error('Engine frame path escaped its job output');
    const file = await open(frame.path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
    try {
      const stat = await file.stat();
      totalBytes += stat.size;
      if (!stat.isFile() || stat.size !== frame.byteLength || totalBytes > maxBytes || stat.size < 24)
        throw new Error('Engine frame file does not match bounded returned inventory');
      const hash = createHash('sha256'), header = Buffer.alloc(24), chunk = Buffer.alloc(64 * 1024);
      let offset = 0;
      while (offset < stat.size) {
        const { bytesRead } = await file.read(chunk, 0, Math.min(chunk.length, stat.size - offset), offset);
        if (!bytesRead) throw new Error('Engine frame file changed during inventory validation');
        hash.update(chunk.subarray(0, bytesRead));
        if (offset < header.length) chunk.copy(header, offset, 0, Math.min(bytesRead, header.length - offset));
        offset += bytesRead;
      }
      const after = await file.stat();
      if (after.size !== stat.size || after.mtimeMs !== stat.mtimeMs || hash.digest('hex') !== frame.sha256 || header.toString('hex', 0, 8) !== '89504e470d0a1a0a' || header.readUInt32BE(16) !== frame.width || header.readUInt32BE(20) !== frame.height)
        throw new Error('Engine frame PNG failed inventory validation');
    } finally { await file.close(); }
  }
  await assertNoSymlinkPath(outputDirectory);
}

async function verifySourceIdentity(source: { videoPath: string; identity: { device: string; inode: string; size: number; modifiedNs: string } }) {
  await assertNoSymlinkPath(source.videoPath);
  const file = await open(source.videoPath, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
  try {
    const info = await file.stat({ bigint: true });
    if (!info.isFile() || info.dev.toString() !== source.identity.device || info.ino.toString() !== source.identity.inode ||
        info.size.toString() !== String(source.identity.size) || info.mtimeNs.toString() !== source.identity.modifiedNs)
      throw new Error('Source video identity changed during frame extraction');
  } finally { await file.close(); }
}

export async function decodeFrames(client: MediaClient, bundleInput: string, runtimePath: string, selection: FrameSelection,
  observeCancel: () => boolean = () => false, testHarness?: { allowTestDouble: true }): Promise<{ jobId: string; bundlePath: string; outputDirectory: string; frames: MediaFrameRef[] }> {
  if (!Number.isSafeInteger(selection.fromMs) || selection.fromMs < 0 || !Number.isSafeInteger(selection.toMs) || selection.toMs <= selection.fromMs ||
      !Number.isSafeInteger(selection.fps) || selection.fps < 1 || selection.fps > MAX_FPS || selection.requestedBundleMs.length < 1 ||
      selection.requestedBundleMs.length > MAX_FRAMES || !selection.requestedBundleMs.every(time => Number.isSafeInteger(time) && time >= selection.fromMs && time < selection.toMs))
    throw new Error('Invalid frame selection; timestamps must be safe slots within a positive bounded range');
  return decodeFrameRequests(client, bundleInput, runtimePath, selection.requestedBundleMs, observeCancel, testHarness);
}

/** Decode caller-selected ordered timestamps exactly, including an endpoint and duplicates. */
export async function decodeFrameTimes(client: MediaClient, bundleInput: string, runtimePath: string, requestedBundleMs: number[],
  observeCancel: () => boolean = () => false, testHarness?: { allowTestDouble: true }) {
  if (requestedBundleMs.length < 1 || requestedBundleMs.length > MAX_FRAMES ||
      !requestedBundleMs.every(time => Number.isSafeInteger(time) && time >= 0))
    throw new Error(`Frame request must contain 1..${MAX_FRAMES} safe timestamps`);
  return decodeFrameRequests(client, bundleInput, runtimePath, requestedBundleMs, observeCancel, testHarness);
}

async function decodeFrameRequests(client: MediaClient, bundleInput: string, runtimePath: string, requestedBundleMs: number[],
  observeCancel: () => boolean, testHarness?: { allowTestDouble: true }): Promise<{ jobId: string; bundlePath: string; outputDirectory: string; frames: MediaFrameRef[] }> {
  const handshake = await client.handshake();
  if ((handshake.implementation !== 'native' && !(testHarness?.allowTestDouble === true && handshake.implementation === 'test-double')) ||
      (handshake.implementation === 'native' && !handshake.capabilities.decodeFrames))
    throw new Error('Installed Engine media provider is unavailable; native frame decoding is not implemented');
  const input = await sourceFor(bundleInput);
  const jobId = createMediaJobId();
  const outputDirectory = await freshJobOutput(runtimePath, jobId);
  const request = { jobId, source: input.source, requestedBundleMs: [...requestedBundleMs], outputDirectory,
    limits: { maxFrames: MAX_FRAMES, maxEdge: 1280, maxTotalPixels: 64_000_000, deadlineMs: 30_000 } } as Omit<DecodeRequest, 'channel'|'protocolVersion'|'clientId'|'rpcId'|'method'>;
  let result: DecodeResult;
  let decodeSettled = false;
  const cancellation = (async () => {
    while (!decodeSettled && !observeCancel()) await new Promise(resolvePromise => setTimeout(resolvePromise, 100));
    if (!decodeSettled && observeCancel()) {
      const state = await client.inspect(request).catch(() => undefined);
      if (state && (state.state === 'accepted' || state.state === 'decoding')) await client.cancel(jobId, 5_000).catch(() => undefined);
      throw new Error('Frame cancellation requested; reconciling the owned job');
    }
  })();
  try {
    const decoded = await Promise.race([client.decode(request), cancellation]);
    if (!decoded) throw new Error('Frame decode did not return a complete inventory');
    result = decoded; decodeSettled = true; await cancellation;
  }
  catch (decodeError) {
    decodeSettled = true;
    await cancellation.catch(() => undefined);
    // RPC timeout/disconnect leaves job state unknown. Recover only while this connection lives.
    try {
      let status = await client.inspect(request);
      for (let i = 0; i < 3 && (status.state === 'accepted' || status.state === 'decoding' || status.state === 'cancelling'); i++) {
        if (observeCancel() && status.state !== 'cancelling') await client.cancel(jobId, 5_000);
        await new Promise(resolvePromise => setTimeout(resolvePromise, 250));
        status = await client.inspect(request);
      }
      if (status.state === 'complete') result = status;
      else if (status.state === 'cancelling' || status.state === 'accepted' || status.state === 'decoding') {
        if (observeCancel() && status.state !== 'cancelling') await client.cancel(jobId, 5_000);
        throw new Error(`Frame job ${jobId} remains ${status.state}; same-connection inspect is required; diagnostic output ${outputDirectory} retained`);
      } else throw new Error(`Frame job ${jobId} ended ${status.state}; diagnostic output ${outputDirectory} retained`);
  } catch (recoveryError) {
      if (recoveryError instanceof Error && /Frame job [0-9a-f]{32} (?:ended (?:failed|cancelled)|remains (?:accepted|decoding|cancelling))/.test(recoveryError.message)) throw recoveryError;
      throw new Error(`Frame job ${jobId} outcome is uncertain; diagnostic output ${outputDirectory} retained (${recoveryError instanceof Error ? recoveryError.message : 'inspection failed'}; decode: ${decodeError instanceof Error ? decodeError.message : 'failed'})`);
    }
  }
  const frames = validateInventory(result.frames, { ...request, channel: 'media', protocolVersion: 1, clientId: '', rpcId: '', method: 'decode' } as DecodeRequest);
  await verifySourceIdentity(input.source);
  await verifyFrameFiles(frames, outputDirectory);
  return { jobId, bundlePath: input.bundlePath, outputDirectory, frames };
}

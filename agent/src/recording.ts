import { basename, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import type { EngineClient, Receipt, StartRequest } from '@apparent/engine-sdk';

export type RecordingClient = Pick<EngineClient, 'handshake' | 'prepareStart' | 'start' | 'control'>;

export const DEFAULT_RECORD_DURATION_SECONDS = 5;

/** Foreground lifecycle; the host owns durable authority storage and child cleanup. */
export async function recordWithEngine(
  client: RecordingClient, out: string, durationSeconds: number,
  interrupted: () => boolean = () => false,
  persist: (request: StartRequest) => Promise<void> = async () => {},
): Promise<string> {
  const ready = await client.handshake(5000);
  if (ready.readiness !== 'ready' || ready.implementation !== 'native' || !ready.capabilities.screenCapture)
    throw new Error('Engine has no ready native screen recorder');
  if (interrupted()) throw new Error('Recording interrupted before start; no capture was requested');
  const request = client.prepareStart(basename(out).replace(/\.apparent$/, ''), resolve(out));
  await persist(request); // Retain authority before an ambiguous start or client crash.
  const capture = await client.start(request, 5000);
  let receipt: Receipt = capture.receipt;
  const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
  const inspect = () => client.control('inspect', capture.captureId, capture.ownerToken, 3000);
  const control = (method: 'stop' | 'cancel') => client.control(method, capture.captureId, capture.ownerToken, 5000);
  const startupBy = performance.now() + 30_000;
  while (receipt.state === 'starting' && performance.now() < startupBy && !interrupted()) {
    await delay(200);
    receipt = await inspect();
  }
  if (receipt.state === 'starting') {
    receipt = await control('cancel');
    if (!interrupted()) throw new Error(`Engine readiness timed out (state: ${receipt.state})`);
  }
  if (receipt.state === 'recording') {
    // Duration begins after observed native readiness, not during startup/portal consent.
    const stopAt = performance.now() + durationSeconds * 1000;
    while (performance.now() < stopAt && !interrupted()) {
      await delay(Math.min(250, Math.max(1, stopAt - performance.now())));
      receipt = await inspect();
      if (receipt.state !== 'recording') break;
    }
    if (receipt.state === 'recording') receipt = await control('stop');
  }
  const finishBy = performance.now() + 60_000;
  while (true) {
    if (receipt.state === 'saved') {
      if (receipt.artifactProvenance !== 'native' || receipt.artifactVersion !== 1 || receipt.bundlePath !== resolve(out))
        throw new Error('Engine saved receipt does not identify the requested native artifact');
      return receipt.bundlePath;
    }
    if (['failed', 'cancelled', 'rejected'].includes(receipt.state))
      throw new Error(`Engine recording ended in state ${receipt.state}`);
    if (performance.now() >= finishBy) throw new Error('Engine finalization timed out; inspect the retained operation before retrying');
    await delay(200);
    receipt = await inspect();
  }
}

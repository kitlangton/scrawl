import { basename, resolve } from 'node:path';
import type { EngineClient, Receipt, StartResult } from '@apparent/engine-sdk';

export type RecordingClient = Pick<EngineClient, 'handshake' | 'prepareStart' | 'start' | 'control'>;

/** Foreground v1 lifecycle. The caller owns process launch, signals and bounded child cleanup. */
export async function recordWithEngine(client: RecordingClient, out: string, durationSeconds: number, interrupted: () => boolean = () => false): Promise<string> {
  const ready = await client.handshake(5000);
  if (ready.readiness !== 'ready' || ready.implementation !== 'native' || !ready.capabilities.screenCapture)
    throw new Error('Engine has no ready native screen recorder');
  if (interrupted()) throw new Error('Recording interrupted before start; no capture was requested');
  const request = client.prepareStart(basename(out).replace(/\.apparent$/, ''), resolve(out));
  const capture: StartResult = await client.start(request, 5000);
  let receipt: Receipt = capture.receipt;
  const deadline = Date.now() + durationSeconds * 1000;
  const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
  if (interrupted()) await stopOrCancel(client, capture);
  while (receipt.state === 'starting' && Date.now() < deadline && !interrupted()) {
    await delay(200);
    receipt = await client.control('inspect', capture.captureId, capture.ownerToken, 3000);
  }
  if (receipt.state === 'starting' && interrupted()) {
    await stopOrCancel(client, capture);
    receipt = await client.control('inspect', capture.captureId, capture.ownerToken, 3000);
  }
  if (receipt.state !== 'recording') {
    if (receipt.state === 'starting') await client.control('cancel', capture.captureId, capture.ownerToken, 3000);
    throw new Error(`Engine did not become ready to record (state: ${receipt.state})`);
  }
  while (Date.now() < deadline && !interrupted()) await delay(Math.min(250, Math.max(1, deadline - Date.now())));
  await client.control('stop', capture.captureId, capture.ownerToken, 5000);
  const finishBy = Date.now() + 60_000;
  while (Date.now() < finishBy) {
    receipt = await client.control('inspect', capture.captureId, capture.ownerToken, 5000);
    if (receipt.state === 'saved') return receipt.bundlePath!;
    if (['failed', 'cancelled', 'rejected'].includes(receipt.state)) throw new Error(`Engine recording ended in state ${receipt.state}`);
    await delay(200);
  }
  throw new Error('Engine finalization timed out; operation may still be active');
}

async function stopOrCancel(client: RecordingClient, capture: StartResult): Promise<void> {
  try { await client.control('stop', capture.captureId, capture.ownerToken, 5000); }
  catch (error) {
    if ((error as {code?: string}).code !== 'not_ready') throw error;
    await client.control('cancel', capture.captureId, capture.ownerToken, 5000);
  }
}

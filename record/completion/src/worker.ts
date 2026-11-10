// One finalized capture, one optional clipboard capability, then exit. No Electron or DOM.
import {begin, reduce} from './policy.ts';
import type {Context, Outcome} from './policy.ts';
import {createInterface} from 'node:readline';

const lines = createInterface({input: process.stdin, crlfDelay: Infinity})[Symbol.asyncIterator]();
let pendingBytes = 0;
process.stdin.on('data', (chunk: Buffer) => {
  for (const byte of chunk) {
    if (++pendingBytes > 32_768) process.exit(1);
    if (byte === 10) pendingBytes = 0;
  }
});
const deadline = setTimeout(() => process.exit(1), 10_000);
async function read(): Promise<Record<string, unknown>> {
  const line = await lines.next();
  if (line.done || Buffer.byteLength(line.value) > 32_768) throw Error('Completion input unavailable');
  return JSON.parse(line.value);
}
function emit(value: unknown): void {process.stdout.write(JSON.stringify(value) + '\n');}
try {
  const input = await read();
  const context = input.context as Context;
  // The parent supplies facts, never commands or paths selected by a renderer.
  if (input.version !== 1 || typeof input.current !== 'boolean' || !context
    || !context.capture || !/^[0-9]+$/.test(context.capture.generation)
    || typeof context.capture.captureId !== 'string' || !context.capture.captureId
    || !(context.capture.requestId === null || typeof context.capture.requestId === 'string')
    || !['linux', 'macos'].includes(context.platform) || !['screen', 'phone'].includes(context.source)
    || ['agentRequested', 'automaticTrialEnabled', 'lab', 'hyprland', 'wayland'].some(k => typeof context[k as keyof Context] !== 'boolean')) {
    throw Error('Invalid completion context');
  }
  const outcome = input.outcome as Outcome;
  if (!outcome || !(outcome.kind === 'saved' && typeof outcome.bundlePath === 'string' && outcome.bundlePath.startsWith('/')
    || outcome.kind === 'failed' && typeof outcome.message === 'string')) throw Error('Invalid finalizer outcome');
  let decision = reduce(begin(context, input.current), {type: 'finalized', capture: context.capture, outcome});
  emit(decision);
  if (decision.effects.length) {
    const reply = await read();
    if (reply.result === 'revoked') decision = reduce(decision.state, {type: 'revoke'});
    else if (reply.result === 'copied' || reply.result === 'failed') {
      decision = reduce(decision.state, {type: 'clipboard', capture: context.capture, result: reply.result});
    } else throw Error('Invalid clipboard receipt');
    emit(decision);
  }
} catch {
  // Do not expose private paths or finalization errors on stderr.
  process.exitCode = 1;
} finally {
  clearTimeout(deadline);
  process.stdin.destroy();
}

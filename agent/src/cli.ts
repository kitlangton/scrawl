#!/usr/bin/env node
import { lstat } from 'node:fs/promises';
import { retainRequest, loadRequest } from './ownership.js';
import { markEntries, markLine } from './inspection.js';
import { isAbsolute, resolve } from 'node:path';
import { resolveBundle, readManifest, readBundle, readRegular, checkImageReference } from './bundle.js';
import { recordWithEngine } from './recording.js';
import { semanticSummary } from './summary.js';

const VERSION = '0.1.0';
function help() { process.stdout.write(`apparent-agent ${VERSION}\nLocal Apparent bundle inspection and Engine foreground recording.\n\nUsage: apparent-agent inspect [BUNDLE|latest] [--json|--marks|--transcript|--images]\n       apparent-agent inspect [BUNDLE|latest] [--experiment agentContext] [--stored-summary]\n       apparent-agent record --duration <seconds> --out <absolute.apparent-path>\n       apparent-agent capture <inspect|stop|cancel> <capture-id>\n       apparent-agent --version\n\nBUNDLE may be a local .apparent directory or apparent.json. URLs are never fetched.\nDefault inspect output is generated from the manifest; --stored-summary prints producer text.\nStorage lookup: APPARENT_CAPTURES_DIR, then capturesDir in ~/.config/apparent/config.json.\nRecording requires APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME absolute paths.\n`); }
async function main(args: string[]) {
  if (args.length === 0 || args.includes('--help') || args.includes('-h')) { help(); return 0; }
  if (args.length === 1 && ['--version', '-V'].includes(args[0])) { console.log(VERSION); return 0; }
  if (args[0] === 'capture') return recover(args.slice(1));
  if (args[0] === 'record') return record(args.slice(1));
  if (args[0] !== 'inspect') throw new Error(`Unknown command: ${args[0]}`);
  let input = 'latest'; let mode = 'summary'; let seenInput = false; let json = false; let storedSummary = false;
  const experiments: string[] = [];
  const rest = args.slice(1);
  for (let i = 0; i < rest.length; i++) {
    const arg = rest[i];
    if (arg === '--json') { if (json) throw new Error('Duplicate --json'); json = true; }
    else if (['--marks', '--transcript', '--images'].includes(arg)) {
      if (mode !== 'summary') throw new Error('Inspection output options conflict');
      mode = arg.slice(2);
    }
    else if (arg === '--experiment') {
      const name = rest[++i];
      if (!name || name.startsWith('-')) throw new Error('--experiment requires a name');
      experiments.push(name);
    }
    else if (arg === '--stored-summary') { if (storedSummary) throw new Error('Duplicate --stored-summary'); storedSummary = true; }
    else if (arg === '--') { const operand = rest.slice(i + 1); if (operand.length !== 1 || seenInput) throw new Error('Only one bundle path is allowed'); input = operand[0]; seenInput = true; break; }
    else if (arg.startsWith('-')) throw new Error(`Unknown option: ${arg}`);
    else if (!seenInput) { input = arg; seenInput = true; }
    else throw new Error('Only one bundle path is allowed');
  }
  if (json && !['summary', 'marks'].includes(mode)) throw new Error('--json supports manifest or marks output');
  if (storedSummary && (mode !== 'summary' || json)) throw new Error('--stored-summary conflicts with other inspection output options');
  const path = await resolveBundle(input);
  const evidence = await lstat(`${path}/.evidence-only.json`).catch((error: NodeJS.ErrnoException) => {
    if (error.code !== 'ENOENT') throw error;
    return null;
  });
  if (evidence?.isFile() && !evidence.isSymbolicLink())
    process.stderr.write('Evidence snapshot: video/audio not downloaded. This local inspection does not fetch media.\n');
  if (json && mode === 'summary') {
    const { raw } = await readManifest(path);
    process.stdout.write(raw);
    return 0;
  }
  const bundle = await readBundle(path);
  if (mode === 'marks') {
    const entries = markEntries(bundle);
    if (json) console.log(JSON.stringify(entries, null, 2));
    else process.stdout.write(entries.length ? entries.map(markLine).join('\n') + '\n' : 'No marks\n');
    return 0;
  }
  if (mode === 'transcript') { process.stdout.write(`${bundle.transcript.text}\n`); return 0; }
  if (mode === 'images') {
    const refs = [bundle.contactSheet, ...(bundle.autocrop ? [bundle.autocrop.fullContactSheet] : []),
      ...bundle.keyframes.flatMap((frame: any) => [frame.file, ...(bundle.autocrop && frame.file.startsWith('frames/') ? [`${bundle.autocrop.fullFrames}/${frame.file.slice('frames/'.length)}`] : [])])]
      .filter((x) => typeof x === 'string');
    for (const ref of refs) {
      await checkImageReference(path, ref);
    }
    process.stdout.write(`${JSON.stringify({ bundle: path, references: [...new Set(refs)] }, null, 2)}\n`);
    return 0;
  }
  if (storedSummary) {
    const summary = new TextDecoder('utf-8', {fatal:true}).decode(await readRegular(`${path}/summary.md`, 64 * 1024 * 1024));
    process.stdout.write(summary.endsWith('\n') ? summary : `${summary}\n`);
    return 0;
  }
  process.stdout.write(semanticSummary(bundle, path, experiments.includes('agentContext'), evidence?.isFile() === true));
  return 0;
}
async function recover(args: string[]) {
  const [method, captureId] = args;
  if (args.length !== 2 || !['inspect', 'stop', 'cancel'].includes(method)) throw new Error('capture requires inspect, stop or cancel and a capture ID');
  const executable = process.env.APPARENT_ENGINE_EXECUTABLE, runtime = process.env.APPARENT_ENGINE_RUNTIME;
  if (!executable || !runtime || !isAbsolute(executable) || !isAbsolute(runtime)) throw new Error('Set explicit absolute Engine executable and runtime paths');
  const request = await loadRequest(runtime, captureId);
  const sdk = await import('@apparent/engine-sdk');
  const client = new sdk.EngineClient(sdk.launchEngine(executable, runtime, 12_000));
  try {
    const handshake = await client.handshake();
    if (handshake.implementation !== 'native') throw new Error('Recovery requires a native Engine');
    const receipt = await client.control(method as 'inspect' | 'stop' | 'cancel', captureId, request.ownerToken);
    process.stdout.write(JSON.stringify(receipt) + '\n');
    return ['failed', 'cancelled', 'rejected'].includes(receipt.state) && method !== 'cancel' ? 1 : 0;
  } finally {
    const closed = await client.close();
    if (!['already-exited', 'eof-exited'].includes(closed)) process.stderr.write('Engine process cleanup did not establish durable completion.\n');
  }
}
async function record(args: string[]) {
  let duration: number | undefined; let out: string | undefined;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--duration' && duration === undefined) duration = Number(args[++i]);
    else if (args[i] === '--out' && out === undefined) out = args[++i];
    else throw new Error(`Unknown option: ${args[i]}`);
  }
  if (!Number.isFinite(duration) || duration! <= 0 || duration! > 3600 || !out || !resolve(out).endsWith('.apparent'))
    throw new Error('record requires --duration (greater than 0 and at most 3600 seconds) and an absolute --out path ending in .apparent');
  if (!isAbsolute(out!)) throw new Error('--out must be absolute');
  const executable = process.env.APPARENT_ENGINE_EXECUTABLE; const runtime = process.env.APPARENT_ENGINE_RUNTIME;
  if (!executable || !runtime || !isAbsolute(executable) || !isAbsolute(runtime))
    throw new Error('Set APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME to explicit absolute paths');
  if (await lstat(out!).catch((error: NodeJS.ErrnoException) => { if (error.code !== 'ENOENT') throw error; return null; })) throw new Error('--out must not already exist');
  const sdk = await import('@apparent/engine-sdk');
  const transport = sdk.launchEngine(resolve(executable), resolve(runtime), 12_000);
  let signalRequested = false;
  const client = new sdk.EngineClient(transport);
  const onSignal = () => { signalRequested = true; };
  process.on('SIGINT', onSignal);
  process.on('SIGTERM', onSignal);
  let recoveryPath: string | undefined;
  try {
    process.stdout.write(`${await recordWithEngine(client, resolve(out!), duration!, () => signalRequested, async request => { recoveryPath = await retainRequest(resolve(runtime), request); })}\n`);
    return 0;
  } catch (error) {
    if (recoveryPath) process.stderr.write(`Private operation recovery record: ${recoveryPath}\n`);
    throw error;
  } finally {
    try {
      const closed = await client.close();
      if (!['already-exited', 'eof-exited'].includes(closed)) process.stderr.write('Engine required forced process cleanup; durable session outcome is unverified.\n');
    } finally { process.removeListener('SIGINT', onSignal); process.removeListener('SIGTERM', onSignal); }
  }
}
main(process.argv.slice(2)).then((code) => { process.exitCode = code; }).catch((error: unknown) => {
  process.stderr.write(`apparent-agent: ${error instanceof Error ? error.message : 'operation failed'}\n`);
  process.exitCode = 2;
});

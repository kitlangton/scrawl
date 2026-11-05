#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import { isAbsolute, resolve } from 'node:path';
import { resolveBundle, readManifest, readBundle } from './bundle.js';
import { recordWithEngine } from './recording.js';

const VERSION = '0.1.0';
function help() { process.stdout.write(`apparent-agent ${VERSION}\nLocal Apparent bundle inspection and Engine foreground recording.\n\nUsage: apparent-agent inspect [BUNDLE|latest] [--json|--marks|--transcript|--images]\n       apparent-agent record --duration <seconds> --out <absolute.apparent-path>\n       apparent-agent --version\n\nBUNDLE may be a local .apparent directory or apparent.json. URLs are never fetched.\nStorage lookup: APPARENT_CAPTURES_DIR, then capturesDir in ~/.config/apparent/config.json.\nRecording requires APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME absolute paths.\n`); }
function marks(bundle: any) {
  return bundle.marks.map((mark: any) => {
    const frame = bundle.keyframes.find((k: any) => k.markId === mark.id || (mark.frame && mark.frame === k.file));
    const same = frame ? bundle.marks.filter((m: any) => m.strokes.length > 0 && (m.id === frame.markId || (m.frame && m.frame === frame.file)))
      .sort((a: any, b: any) => a.t0 - b.t0) : [];
    const number = Math.max(1, same.findIndex((m: any) => m.id === mark.id) + 1);
    return { keyframe: frame?.index ?? null, number, numbered: same.length >= 2, id: mark.id, t0: mark.t0,
      t1: mark.t1, time: `${Math.floor(mark.t0 / 60000)}:${String(Math.floor(mark.t0 / 1000) % 60).padStart(2, '0')}`,
      shape: mark.shape, on: mark.on ?? null, strokes: mark.strokes?.length ?? 0, text: mark.description, frame: mark.frame ?? null };
  });
}
async function main(args: string[]) {
  if (args.length === 0 || args.includes('--help') || args.includes('-h')) { help(); return 0; }
  if (args.length === 1 && ['--version', '-V'].includes(args[0])) { console.log(VERSION); return 0; }
  if (args[0] === 'record') return record(args.slice(1));
  if (args[0] !== 'inspect') throw new Error(`Unknown command: ${args[0]}`);
  let input = 'latest'; let mode = 'summary'; let seenInput = false;
  for (const arg of args.slice(1)) {
    if (['--json', '--marks', '--transcript', '--images'].includes(arg)) {
      if (mode !== 'summary') throw new Error('Inspection output options conflict');
      mode = arg.slice(2);
    }
    else if (arg === '--') { const rest = args.slice(args.indexOf(arg) + 1); if (rest.length !== 1 || seenInput) throw new Error('Only one bundle path is allowed'); input = rest[0]; seenInput = true; break; }
    else if (arg.startsWith('-')) throw new Error(`Unknown option: ${arg}`);
    else if (!seenInput) { input = arg; seenInput = true; }
    else throw new Error('Only one bundle path is allowed');
  }
  const path = await resolveBundle(input);
  if (mode === 'json') {
    const { raw } = await readManifest(path);
    process.stdout.write(raw);
    return 0;
  }
  const bundle = await readBundle(path);
  if (mode === 'marks') { console.log(JSON.stringify(marks(bundle), null, 2)); return 0; }
  if (mode === 'transcript') { process.stdout.write(`${bundle.transcript.text}\n`); return 0; }
  if (mode === 'images') {
    const refs = [bundle.contactSheet, ...(bundle.autocrop ? [bundle.autocrop.fullContactSheet] : []),
      ...bundle.keyframes.flatMap((frame: any) => [frame.file, ...(bundle.autocrop && frame.file.startsWith('frames/') ? [`${bundle.autocrop.fullFrames}/${frame.file.slice('frames/'.length)}`] : [])])]
      .filter((x) => typeof x === 'string');
    for (const ref of refs) {
      if (ref.includes('\\') || ref.includes('\0') || ref.startsWith('/') || /^[a-z][a-z\d+.-]*:/i.test(ref) || ref.split('/').includes('..')) throw new Error('Bundle contains an unsafe image reference');
    }
    process.stdout.write(`${JSON.stringify({ bundle: path, references: [...new Set(refs)] }, null, 2)}\n`);
    return 0;
  }
  // summary.md is the producer-authored semantic handoff. Do not regenerate it here.
  let summary: string;
  try { summary = await readFile(`${path}/summary.md`, 'utf8'); }
  catch { throw new Error(`Stored summary.md is missing or unreadable in ${path}; use --transcript or --json for available manifest data`); }
  process.stdout.write(summary.endsWith('\n') ? summary : `${summary}\n`);
  return 0;
}
async function record(args: string[]) {
  let duration: number | undefined; let out: string | undefined;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--duration') duration = Number(args[++i]);
    else if (args[i] === '--out') out = args[++i];
    else throw new Error(`Unknown option: ${args[i]}`);
  }
  if (!Number.isFinite(duration) || duration! <= 0 || duration! > 3600 || !out || !resolve(out).endsWith('.apparent'))
    throw new Error('record requires --duration (greater than 0 and at most 3600 seconds) and an absolute --out path ending in .apparent');
  if (!isAbsolute(out!)) throw new Error('--out must be absolute');
  const executable = process.env.APPARENT_ENGINE_EXECUTABLE; const runtime = process.env.APPARENT_ENGINE_RUNTIME;
  if (!executable || !runtime || !isAbsolute(executable) || !isAbsolute(runtime))
    throw new Error('Set APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME to explicit absolute paths');
  const sdk = await import('@apparent/engine-sdk');
  const transport = sdk.launchEngine(resolve(executable), resolve(runtime));
  let signalRequested = false;
  const client = new sdk.EngineClient(transport);
  const onSignal = () => { signalRequested = true; };
  process.once('SIGINT', onSignal);
  process.once('SIGTERM', onSignal);
  try {
    process.stdout.write(`${await recordWithEngine(client, resolve(out!), duration!, () => signalRequested)}\n`);
    return 0;
  } finally { process.removeListener('SIGINT', onSignal); process.removeListener('SIGTERM', onSignal); await client.close(); }
}
main(process.argv.slice(2)).then((code) => { process.exitCode = code; }).catch((error: unknown) => {
  process.stderr.write(`apparent-agent: ${error instanceof Error ? error.message : 'operation failed'}\n`);
  process.exitCode = 2;
});

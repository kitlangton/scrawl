#!/usr/bin/env node
import { lstat, readFile } from 'node:fs/promises';
import { retainRequest, loadRequest } from './ownership.js';
import { markEntries, markLine } from './inspection.js';
import { isAbsolute, resolve } from 'node:path';
import { resolveBundle, readManifest, readBundle, readRegular, checkImageReference, hasMicrophoneSamples, configuredAgentContext } from './bundle.js';
import { recordWithEngine } from './recording.js';
import { controlCapture2, discoverCapture2Sources, recordWithCapture2 } from './recording/capture2.js';
import { semanticSummary } from './summary.js';
import { issueCommand, issueHelp, IssueCliError } from './issue.js';
import { decodeFrames, hasSourceVideo, savedKeyframes, selectFrameTimes, verifyFramesExecutable, verifyPrivateRuntime } from './frames.js';
import { runZoom } from './zoom/zoom.js';
import { generateDemo } from './demo/generate.js';

const VERSION = '0.1.0';
function help() { process.stdout.write(`apparent-agent ${VERSION}\nLocal Apparent bundle inspection, synthetic demo generation, and Engine foreground recording/frame selection.\n\nUsage: apparent-agent inspect [BUNDLE|latest] [--json|--marks|--transcript|--images]\n       apparent-agent inspect [BUNDLE|latest] [--experiment agentContext] [--stored-summary]\n       apparent-agent demo [--out FRESH.apparent] [--json]\n       apparent-agent frames BUNDLE --from TIME --to TIME [--fps 4] [--json]\n       apparent-agent zoom [BUNDLE|latest] (--at TIME_OR_PHRASE | --mark NUMBER_OR_ID) [--frame N] [--span S] [--frames N] [--full] [--out DIR] [--json]\n       apparent-agent skill\n       apparent-agent record --duration <seconds> --out <absolute.apparent-path>\n       apparent-agent record --protocol capture2 --duration <seconds> --out <absolute.apparent-path> --source <discovered-id>\n       apparent-agent capture <inspect|stop|cancel> <capture-id>\n       apparent-agent capture --protocol capture2 <inspect|stop|cancel> <request-id>\n       apparent-agent capture --protocol capture2 sources [--json]\n       apparent-agent issue --title <TEXT> (--body <TEXT> | --body-file <PATH>) [--submit]\n       apparent-agent --version\n\nBUNDLE may be a local .apparent directory or apparent.json. URLs are never fetched.\nDefault inspect output is generated from the manifest; --stored-summary prints producer text.\nDemo creates illustrative synthetic local data only; it does not record a screen or audio.\nStorage lookup: APPARENT_CAPTURES_DIR, then capturesDir in ~/.config/apparent/config.json.\nRecording requires APPARENT_ENGINE_EXECUTABLE and APPARENT_ENGINE_RUNTIME absolute paths.\nCapture/2 requires explicit --protocol capture2 and APPARENT_ENGINE_CAPTURE_EXECUTABLE; it never falls back to V1. capture --protocol capture2 sources [--json] is read-only public SDK discovery.\nFrames require APPARENT_ENGINE_FRAMES_EXECUTABLE and APPARENT_ENGINE_RUNTIME absolute paths.\nIssue previews are offline by default; --submit publishes approved text to tryapparent/apparent.\n`); }
async function main(args: string[]) {
  if(args[0]==='demo'&&args.length===2&&['--help','-h'].includes(args[1])) { process.stdout.write('Usage: apparent-agent demo [--out FRESH.apparent] [--json] [--clipboard]\nGenerate a local illustrative synthetic bundle; no capture, Engine, Desktop, provider, or network is used. --clipboard currently fails before writing until a native clipboard route is published.\n'); return 0; }
  if (args[0] === 'zoom' && args.length === 2 && ['--help','-h'].includes(args[1])) {
    process.stdout.write('Usage: apparent-agent zoom [BUNDLE|latest] (--at TIME_OR_PHRASE | --mark NUMBER_OR_ID) [--frame N] [--span S] [--frames N] [--full] [--out DIR] [--json]\n');
    return 0;
  }
  if (args[0] !== 'demo' && args[0] !== 'skill' && args[0] !== 'issue' && (args.length === 0 || args.includes('--help') || args.includes('-h'))) { help(); return 0; }
  if (args.length === 1 && ['--version', '-V'].includes(args[0])) { console.log(VERSION); return 0; }
  if (args[0] === 'skill') {
    if (args.length !== 1) throw new Error('skill takes no operands or options');
    const guide = await readFile(new URL('../skills/apparent/SKILL.md', import.meta.url));
    process.stdout.write(guide);
    if (guide.length === 0 || guide[guide.length - 1] !== 0x0a) process.stdout.write('\n');
    return 0;
  }
  if (args[0] === 'issue') {
    const result = await issueCommand(args.slice(1));
    if (result === 'help') process.stdout.write(issueHelp());
    else process.stdout.write(`${JSON.stringify(result)}\n`);
    return 0;
  }
  if (args[0] === 'capture') {
    const captureArgs = args.slice(1);
    if (captureArgs[0] === '--protocol' && captureArgs[1] === 'capture2' && captureArgs[2] === 'sources') return discoverCapture2Sources(captureArgs.slice(3));
    return captureArgs.includes('--protocol') ? controlCapture2(captureArgs) : recover(captureArgs);
  }
  if (args[0] === 'demo') return demo(args.slice(1));
  if (args[0] === 'record') return args.slice(1).includes('--protocol') ? recordWithCapture2(args.slice(1)) : record(args.slice(1));
  if (args[0] === 'frames') return frames(args.slice(1));
  if (args[0] === 'zoom') return zoom(args.slice(1));
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
  if (storedSummary && experiments.includes('agentContext')) throw new Error('--stored-summary cannot apply --experiment agentContext');
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
  const agentContext = experiments.includes('agentContext') || await configuredAgentContext();
  const hasAudio = bundle.source !== 'iphone' && bundle.context?.microphoneCapture !== 'off'
    ? await hasMicrophoneSamples(path, bundle.audio)
    : false;
  process.stdout.write(semanticSummary(bundle, path, agentContext, evidence?.isFile() === true, hasAudio));
  return 0;
}

async function demo(args:string[]) {
  let out:string|undefined,json=false,clipboard=false;
  for(let i=0;i<args.length;i++){
    const arg=args[i];
    if(arg==='--out'&&out===undefined){out=args[++i];if(!out||out.startsWith('--')||!out.endsWith('.apparent'))throw new Error('--out requires a fresh absolute .apparent directory');}
    else if(arg==='--json'&&!json)json=true;
    else if(arg==='--clipboard'&&!clipboard)clipboard=true;
    else throw new Error(`Unknown or duplicate demo option: ${arg}`);
  }
  if(clipboard)throw new Error('demo --clipboard is unavailable: no published native clipboard route');
  const result=await generateDemo(out);
  if(json)process.stdout.write(`${JSON.stringify(result,null,2)}\n`);
  else {process.stdout.write(`${result.summary}\nSynthetic demo bundle: ${result.path}\nImages:\n${result.images.map(image=>`  ${image}`).join('\n')}\n`);}
  return 0;
}
async function zoom(args: string[]) {
  let bundle='latest', seenBundle=false, at: string|undefined, mark: string|undefined, frame: number|undefined;
  let span=2, frames=12, full=false, out: string|undefined, json=false, seenSpan=false, seenFrames=false;
  for(let i=0;i<args.length;i++) {
    const arg=args[i];
    if(arg==='--at'&&at===undefined) { at=args[++i]; if(!at||at.startsWith('--')) throw new Error('--at requires a time or narration phrase'); }
    else if(arg==='--mark'&&mark===undefined) { mark=args[++i]; if(!mark||mark.startsWith('--')) throw new Error('--mark requires a mark number or id'); }
    else if(arg==='--frame'&&frame===undefined) { const value=args[++i]; if(value===undefined||!/^\d+$/.test(value)) throw new Error('--frame requires a positive integer keyframe number'); frame=Number(value); }
    else if(arg==='--span'&&!seenSpan) { seenSpan=true; const value=args[++i]; if(value===undefined) throw new Error('--span requires seconds'); span=Number(value); }
    else if(arg==='--frames'&&!seenFrames) { seenFrames=true; const value=args[++i]; if(value===undefined||!/^\d+$/.test(value)) throw new Error('--frames requires a nonnegative integer'); frames=Number(value); if(!Number.isSafeInteger(frames)) throw new Error('--frames exceeds the safe integer range'); }
    else if(arg==='--full'&&!full) full=true;
    else if(arg==='--out'&&out===undefined) { out=args[++i]; if(!out||out.startsWith('--')) throw new Error('--out requires an absolute fresh directory'); }
    else if(arg==='--json'&&!json) json=true;
    else if(arg==='--'&&!seenBundle&&i+2===args.length) { bundle=args[++i]; seenBundle=true; }
    else if(arg.startsWith('-')) throw new Error(`Unknown or duplicate zoom option: ${arg}`);
    else if(!seenBundle) { bundle=arg; seenBundle=true; }
    else throw new Error('zoom accepts one bundle path');
  }
  const result=await runZoom({bundle,at,mark,frame,span,frames,full,out,json});
  if(json) process.stdout.write(`${JSON.stringify(result,null,2)}\n`);
  else {
    process.stdout.write(`${result.sheet}\n${result.window.from} → ${result.window.to}, ${result.cells.length} frames (${result.sourceKind})\n`);
    for(const cell of result.cells) process.stdout.write(`  ${cell.index}. ${cell.time} → ${cell.framePath}\n`);
    if(result.said) process.stdout.write(`Said: “${result.said}”\n`);
    process.stdout.write(`Details: ${result.sheet.replace(/zoom\.png$/,'zoom.json')}\n`);
  }
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
async function frames(args: string[]) {
  let bundle = 'latest', from: string | undefined, to: string | undefined, fps = 4, json = false, seenBundle = false;
  let seenFps = false;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--from' && from === undefined) from = args[++i];
    else if (arg === '--to' && to === undefined) to = args[++i];
    else if (arg === '--fps' && !seenFps) { seenFps = true; const value = args[++i]; if (value === undefined) throw new Error('--fps requires an integer'); fps = Number(value); }
    else if (arg === '--json' && !json) json = true;
    else if (arg === '--' && !seenBundle && i + 2 === args.length) { bundle = args[++i]; seenBundle = true; }
    else if (arg.startsWith('-')) throw new Error(`Unknown or duplicate frames option: ${arg}`);
    else if (!seenBundle) { bundle = arg; seenBundle = true; }
    else throw new Error('frames accepts one bundle path');
  }
  if (from === undefined || to === undefined) throw new Error('frames requires --from and --to');
  const selection = selectFrameTimes(from, to, fps);
  if (!selection.requestedBundleMs.length) throw new Error('Frame range must select at least one timestamp');
  const executable = process.env.APPARENT_ENGINE_FRAMES_EXECUTABLE, runtime = process.env.APPARENT_ENGINE_RUNTIME;
  const emitSaved = async () => {
    const saved = await savedKeyframes(bundle, selection);
    if (!saved.frames.length) throw new Error('No saved keyframes match this range; video extraction is unavailable without an installed Engine media provider');
    process.stderr.write('Using saved keyframes; no video was decoded.\n');
    if (json) process.stdout.write(`${JSON.stringify({ bundle: saved.bundlePath, mode: 'saved-keyframes', source: 'bundle-manifest', frames: saved.frames }, null, 2)}\n`);
    else for (const frame of saved.frames) process.stdout.write(`${frame.path}\n`);
  };
  if (selection.fromMs === selection.toMs || (!executable && !runtime)) { await emitSaved(); return 0; }
  if (!executable || !runtime || !isAbsolute(executable) || !isAbsolute(runtime) || resolve(executable) !== executable || resolve(runtime) !== runtime)
    throw new Error('Set APPARENT_ENGINE_FRAMES_EXECUTABLE and APPARENT_ENGINE_RUNTIME to explicit canonical absolute paths');
  if (!await hasSourceVideo(bundle)) { await emitSaved(); return 0; }
  await verifyFramesExecutable(executable);
  await verifyPrivateRuntime(runtime);
  const sdk = await import('@apparent/engine-sdk/media');
  const client = new sdk.MediaClient(sdk.launchMediaWorker(executable, runtime, 3000));
  let signalRequested = false;
  const onSignal = () => { signalRequested = true; };
  process.on('SIGINT', onSignal); process.on('SIGTERM', onSignal);
  try {
    const handshake = await client.handshake();
    if (handshake.implementation === 'native' && !handshake.capabilities.decodeFrames) { await emitSaved(); return 0; }
    const result = await decodeFrames(client, bundle, runtime, selection, () => signalRequested);
    if (json) process.stdout.write(`${JSON.stringify({ bundle: result.bundlePath, jobId: result.jobId, outputDirectory: result.outputDirectory,
      mode: 'decoded-video', source: 'engine-media-sdk',
      frames: result.frames.map(frame => ({ ...frame, clamped: frame.clampedToStart || frame.clampedToEnd })) }, null, 2)}\n`);
    else for (const frame of result.frames) process.stdout.write(`${frame.path}\n`);
    return 0;
  } finally {
    const closed = await client.close();
    if (!['already-exited', 'eof-exited'].includes(closed)) process.stderr.write(`Media worker shutdown is uncertain (${closed}); retain job output for diagnosis.\n`);
    process.removeListener('SIGINT', onSignal); process.removeListener('SIGTERM', onSignal);
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
  process.exitCode = error instanceof IssueCliError ? error.exitCode : 2;
});

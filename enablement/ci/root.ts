// CI scheduling only. The verifier supplies the unchanged Cargo command inventory.
import {spawn, type ChildProcess} from 'node:child_process';
import {appendFileSync} from 'node:fs';
import {constants} from 'node:os';
import {performance} from 'node:perf_hooks';
import {StringDecoder} from 'node:string_decoder';
import {fileURLToPath} from 'node:url';

type Commands = {test: string[]; build: string[]};
type Result = {code: number; detail: string};
const script = fileURLToPath(import.meta.url);
const signals = ['SIGINT', 'SIGTERM', 'SIGHUP'] as const;
const status = (code: number | null, signal: NodeJS.Signals | null) =>
  signal ? 128 + constants.signals[signal] : code ?? 1;

function timing(label: string) {
  const started = new Date().toISOString(), tick = performance.now();
  let done = false;
  return (code: number) => {
    if (done) return;
    done = true;
    if (!process.env.APPARENT_TIMING_FILE) return;
    try {
      appendFileSync(process.env.APPARENT_TIMING_FILE, JSON.stringify({schema: 1, label,
        started, elapsed_ms: Math.round(performance.now() - tick), code,
        sha: process.env.GITHUB_SHA ?? null}) + '\n', {mode: 0o600});
    } catch { /* Observations cannot change command acceptance. */ }
  };
}

// This process is the detached session/group leader and stays alive after Cargo
// exits. Only the leader signals its own group: no parent ever signals a reaped
// PID/PGID. Disconnect/cancellation uses the same anchored cleanup boundary.
function supervise(commands: Commands) {
  let finishing = false;
  const cleanup = () => process.kill(-process.pid, 'SIGKILL');
  const finish = (code: number, detail: string) => {
    if (finishing) return;
    finishing = true;
    process.stdout.write('', () => {
      if (!process.connected || !process.send) return cleanup();
      process.send({code, detail}, () => cleanup());
    });
  };
  process.on('disconnect', cleanup);
  process.on('message', message => {
    const signal = (message as {cancel?: NodeJS.Signals})?.cancel;
    if (signal && signals.includes(signal as typeof signals[number]))
      finish(status(null, signal), `Cancelled by ${signal}`);
  });
  signals.forEach(signal => process.on(signal, () => finish(status(null, signal), `Cancelled by ${signal}`)));
  process.on('uncaughtException', error => finish(1, error.stack ?? String(error)));
  process.on('unhandledRejection', error => finish(1, String(error)));

  let compiled = false, refused = false, testCode: number | undefined, buildCode: number | undefined;
  let build: ChildProcess | undefined;
  const compileTiming = timing('rust-test-compilation');
  let testTiming: ((code: number) => void) | undefined;
  const complete = () => {
    if (testCode === 0 && buildCode === 0 && compiled)
      finish(0, 'Root test harness and required app build both succeeded');
  };
  const startBuild = () => {
    if (build || finishing) return;
    console.log('Root test compilation complete; starting one app-build worker alongside two test threads');
    const buildTiming = timing('rust-build');
    build = spawn(commands.build[0], commands.build.slice(1), {stdio: 'inherit'});
    build.once('error', error => { buildTiming(1); finish(1, `App build spawn failed: ${error.message}`); });
    build.once('exit', (code, signal) => {
      buildCode = status(code, signal); buildTiming(buildCode);
      if (buildCode !== 0) finish(buildCode, 'Required app build failed');
      else complete();
    });
  };
  const decoder = new StringDecoder('utf8');
  let buffer = '';
  const line = (value: string) => {
    let event: any;
    try { event = JSON.parse(value); } catch { console.log(value); return; }
    if (event?.reason === 'build-finished') {
      if (event.success !== true) refused = true;
      else if (!compiled && !refused && !finishing) {
        compiled = true; compileTiming(0); testTiming = timing('rust-test-execution'); startBuild();
      }
    } else if (event?.reason === 'compiler-message' && typeof event.message?.rendered === 'string') {
      process.stdout.write(event.message.rendered);
    } else if (!['compiler-artifact', 'build-script-executed'].includes(event?.reason)) console.log(value);
  };
  const consume = (chunk: Buffer) => {
    buffer += decoder.write(chunk);
    let end: number;
    while ((end = buffer.indexOf('\n')) !== -1) {
      const value = buffer.slice(0, end); buffer = buffer.slice(end + 1); line(value);
    }
  };
  const test = spawn(commands.test[0], commands.test.slice(1), {stdio: ['inherit', 'pipe', 'inherit']});
  test.stdout!.on('data', consume);
  test.once('error', error => { compileTiming(1); finish(1, `Test spawn failed: ${error.message}`); });
  test.once('exit', (code, signal) => {
    // Drain available data without awaiting EOF from descendants inheriting the
    // pipe. Their owned group is cleaned once both commands finish or one fails.
    setImmediate(() => {
      test.stdout!.removeListener('data', consume);
      let chunk: Buffer | null;
      while ((chunk = test.stdout!.read()) !== null) consume(chunk);
      buffer += decoder.end();
      if (buffer) line(buffer);
      test.stdout!.destroy();
      testCode = status(code, signal);
      if (!compiled) compileTiming(testCode || 1);
      else testTiming!(testCode);
      if (testCode !== 0) finish(testCode, 'Root tests failed');
      else if (!compiled || refused) finish(1, 'Cargo did not report successful test compilation');
      else complete();
    });
  });
}

async function run(commands: Commands): Promise<Result> {
  return new Promise(resolve => {
    const worker = spawn(process.execPath, ['--experimental-strip-types', script, '--supervise', JSON.stringify(commands)],
      {detached: true, stdio: ['inherit', 'inherit', 'inherit', 'ipc']});
    let result: Result | undefined, cancelled: NodeJS.Signals | undefined;
    worker.on('message', value => {
      const message = value as Result;
      if (Number.isInteger(message?.code) && message.code >= 0 && typeof message.detail === 'string') result = message;
    });
    const handlers = signals.map(signal => {
      const handler = () => {
        cancelled = signal;
        if (worker.connected) worker.send({cancel: signal}, () => {});
      };
      process.on(signal, handler); return handler;
    });
    const remove = () => signals.forEach((signal, index) => process.removeListener(signal, handlers[index]));
    worker.once('error', error => { remove(); resolve({code: 1, detail: `Supervisor spawn failed: ${error.message}`}); });
    worker.once('close', (_code, signal) => {
      remove();
      resolve(cancelled ? {code: status(null, cancelled), detail: `Cancelled by ${cancelled}`}
        : result && signal === 'SIGKILL' ? result
        : {code: 1, detail: 'Supervisor exited without an owned cleanup result'});
    });
  });
}

const worker = process.argv[2] === '--supervise';
const commands = JSON.parse(process.argv[worker ? 3 : 2] ?? 'null') as Commands;
if (!commands || ![commands.test, commands.build].every(command => Array.isArray(command) && command.length > 0
    && command.every(argument => typeof argument === 'string' && !argument.includes('\0'))))
  throw Error('Expected test and build command arrays');
if (worker) {
  if (!process.send || !process.connected) throw Error('Supervisor requires its owning IPC connection');
  supervise(commands);
} else {
  const result = await run(commands);
  console.log(result.detail);
  process.exitCode = result.code;
}

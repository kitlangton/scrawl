// Temporary PR1092 discriminator. This is neither a Storage test nor proof of it.
import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {createHash} from 'node:crypto';
import {once} from 'node:events';
import * as fs from 'node:fs/promises';
import {createRequire} from 'node:module';
import os from 'node:os';
import path from 'node:path';
import {promisify} from 'node:util';
import {panelScript} from './mac-picker.ts';

const exec = promisify(execFile);
const frontend = path.resolve('capture/desktop/ui/frontend');
const require = createRequire(path.join(frontend, 'package.json'));
const deadline = Date.now() + 90000;
const budget = (cap: number, cleanup = false) => {
  const remaining = deadline - Date.now() - (cleanup ? 0 : 15000);
  assert.ok(remaining > 0, 'Probe absolute deadline exceeded');
  return Math.min(cap, remaining);
};
const within = <T>(promise: Promise<T>, cap: number, cleanup = false): Promise<T> => {
  let timer: ReturnType<typeof setTimeout>;
  return Promise.race([promise, new Promise<T>((_, reject) => {
    timer = setTimeout(() => reject(Error('Probe operation deadline exceeded')), budget(cap, cleanup));
  })]).finally(() => clearTimeout(timer));
};

assert.equal(process.platform, 'darwin');
for (const [key, value] of Object.entries({CI: 'true', GITHUB_ACTIONS: 'true',
  RUNNER_ENVIRONMENT: 'github-hosted', RUNNER_OS: 'macOS', APPARENT_PICKER_PROBE_CI: '1'})) {
  assert.equal(process.env[key], value, `Refuse unqualified probe: ${key}`);
}
const runner = await fs.realpath(process.env.RUNNER_TEMP!);
const output = path.join(runner, 'mac-picker-probe');
await fs.mkdir(output, {mode: 0o700});
const reportPath = path.join(output, 'report.json');
const report: any = {scope: 'One synthetic owned chooser; read-only AX diagnosis; no Storage acceptance',
  source: process.env.GITHUB_SHA, run: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT,
  node: process.version, os: os.release(), arch: process.arch,
  panelHash: createHash('sha256').update(panelScript).digest('hex'), probes: [], cleanup: {}};
const save = () => fs.writeFile(reportPath, JSON.stringify(report, null, 2) + '\n', {mode: 0o600});
const saveFinal = async () => {
  try { await save(); } catch (error: any) { report.writeError = error.message; failure ??= error; }
};
// Insert only log statements. Preserve every original source line and query.
const instrumented = panelScript.split('\n').flatMap((line, index) => {
  const query = /^\s*(set|if)\b/.test(line) &&
    /\b(application process|windows?|name|role|buttons|sheets|UI elements|position|size|enabled|frontmost|text fields|value)\b/.test(line);
  return query ? [`${line.match(/^\s*/)![0]}log "AX probe source-line=${index + 1}"`, line] : [line];
}).join('\n');
assert.equal(instrumented.split('\n').filter(line => !line.includes('log "AX probe source-line=')).join('\n'), panelScript);
await fs.writeFile(path.join(output, 'panel.applescript'), instrumented, {mode: 0o600});
let application: any, child: any, home: string | undefined, failure: unknown;
await save();
try {
  // Syntax failure is a diagnostic error, before any native UI is opened.
  await exec('/usr/bin/osacompile', ['-o', path.join(output, 'panel.scpt'), path.join(output, 'panel.applescript')],
    {timeout: budget(3500), killSignal: 'SIGKILL'});
  const permission = await exec('/usr/bin/osascript', ['-e', 'tell application "System Events" to get UI elements enabled'],
    {timeout: budget(3500), killSignal: 'SIGKILL'});
  assert.equal(permission.stdout.trim(), 'true', 'Existing CI accessibility required; never change TCC');
  home = await fs.mkdtemp(path.join(runner, 'picker-probe-'));
  const env: NodeJS.ProcessEnv = {...process.env, HOME: home, CFFIXED_USER_HOME: home,
    XDG_CONFIG_HOME: path.join(home, '.config'), APPARENT_CONFIG: path.join(home, 'config.json'),
    TMPDIR: home, TMP: home, TEMP: home};
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.APPARENT_CAPTURES_DIR;
  const executable = require('electron');
  assert.ok(typeof executable === 'string' && executable.startsWith(frontend + '/node_modules/electron/'));
  await fs.access(executable, fs.constants.X_OK);
  await fs.access(path.join(frontend, 'dist/index.html'));
  const {_electron} = require('@playwright/test');
  application = await _electron.launch({args: [frontend], executablePath: executable, env, timeout: budget(15000)});
  child = application.process();
  const page: any = await within(application.firstWindow(), 5000);
  await page.waitForLoadState('domcontentloaded', {timeout: budget(5000)});
  report.identity = await within(application.evaluate(({app, BrowserWindow}: any) => {
    const windows = BrowserWindow.getAllWindows();
    if (windows.length !== 1 || windows[0].getTitle() !== 'Apparent — UI foundation' || !windows[0].isVisible()) {
      throw Error('Unknown or invisible owned window');
    }
    return {pid: process.pid, electron: process.versions.electron, userData: app.getPath('userData'),
      executable: process.execPath, title: windows[0].getTitle()};
  }), 5000);
  assert.equal(report.identity.pid, child.pid);
  assert.equal(report.identity.executable, executable);
  assert.ok(report.identity.userData.startsWith(home + path.sep), 'Refuse a nonisolated Electron profile');
  await save();
  // One opening only, attached to the verified shipped window. A native event
  // distinguishes a shown sheet from merely issuing an open-dialog request.
  report.sheet = await within(application.evaluate(async ({dialog, BrowserWindow}: any, defaultPath: string) => {
    const windows = BrowserWindow.getAllWindows();
    if (windows.length !== 1 || windows[0].getTitle() !== 'Apparent — UI foundation') throw Error('Owned window changed');
    const parent = windows[0];
    const shown = new Promise((resolve, reject) => {
      parent.once('sheet-begin', () => resolve({event: 'sheet-begin', pid: process.pid}));
      void dialog.showOpenDialog(parent, {title: 'Choose capture location', buttonLabel: 'Choose',
        properties: ['openDirectory', 'createDirectory'], defaultPath,
        message: 'New captures use this location. Existing captures stay where they are.'}).catch(reject);
    });
    return shown;
  }, home), 5000);
  await save();
  // Two read-only observations of that same sheet: retain cold/warm checkpoints,
  // including each failure. No mutation, timeout increase or acceptance fallback.
  for (const label of ['cold', 'warm']) {
    const start = performance.now();
    try {
      const result = await exec('/usr/bin/osascript', ['-e', instrumented, String(child.pid), home, 'inspect'],
        {timeout: budget(3500), killSignal: 'SIGKILL', maxBuffer: 65536});
      report.probes.push({label, elapsedMs: performance.now() - start, ...result});
    } catch (error: any) {
      report.probes.push({label, elapsedMs: performance.now() - start, code: error.code,
        signal: error.signal, killed: error.killed, stdout: error.stdout, stderr: error.stderr});
    }
    await save();
  }
  if (report.probes.some((probe: any) => probe.killed || probe.code !== undefined)) {
    failure = Error('AX probe preserved failed selector observations; inspect report.json');
  }
} catch (error: any) {
  failure = error;
  report.error = {message: error.message, stack: error.stack, stdout: error.stdout, stderr: error.stderr};
} finally {
  // Evidence I/O must never prevent owned-process teardown.
  if (application) {
    try {
      try { await within(application.close(), 3000, true); }
      catch (error: any) { report.cleanup.closeError = error.message; }
      const exited = () => child.exitCode !== null || child.signalCode !== null;
      // As in the native fixture, signal only the exact owned ChildProcess
      // handle before exit. Never signal an unanchored numeric PID or group.
      for (const signal of ['SIGTERM', 'SIGKILL'] as const) {
        if (exited()) break;
        report.cleanup[signal] = child.kill(signal);
        try { await within(once(child, 'exit'), 2000, true); }
        catch (error) { if (signal === 'SIGKILL' && !exited()) throw error; }
      }
      report.cleanup = {exitCode: child.exitCode, signalCode: child.signalCode,
        ...report.cleanup, exited: exited()};
      assert.equal(report.cleanup.exited, true, 'Owned Electron did not exit');
    } catch (error: any) { report.cleanup.error = error.message; failure ??= error; }
  }
  if (home && (!application || report.cleanup.exited)) {
    try {
      await fs.rm(home, {recursive: true, force: true});
      report.cleanup.fixtureRemoved = true;
    } catch (error: any) { report.cleanup.fixtureError = error.message; failure ??= error; }
  }
  await saveFinal();
}
console.log(JSON.stringify(report));
if (failure) throw failure;

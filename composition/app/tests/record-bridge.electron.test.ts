import assert from 'node:assert/strict';
import { test } from 'node:test';
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { _electron as electron, expect } from '@playwright/test';
import { build } from 'esbuild';
import { createRequire } from 'node:module';
const { stopElectron } = createRequire(import.meta.url)('../../../enablement/native/owned-processes.ts') as {
  stopElectron(application: Awaited<ReturnType<typeof electron.launch>> | undefined): Promise<void>;
};
import { APP_SHELL_API, REACT_VERSION, parseStreamManifest } from '@apparent/app-contracts';
import { buildShell, root } from '../scripts/build.ts';
import { assemble, verifyAssembly } from '../scripts/assemble.ts';
import { packageApplication } from '../scripts/package.ts';
import { digest, readRegular } from '../scripts/artifacts.ts';

const external = ['react', 'react/jsx-runtime', 'react-dom/client'];

async function buildSyntheticRecordStream(source: string, output: string) {
  await mkdir(source, { recursive: true });
  await mkdir(output, { recursive: true });
  await writeFile(join(source, 'stream.json'), JSON.stringify({ id: 'record-test', label: 'Record Test', version: '1.0.0' }));
  const adapter = JSON.stringify(join(root, 'src/record/renderer-client.ts'));
  await writeFile(join(source, 'index.tsx'), `import * as React from 'react';
import { connectRecordClient } from ${adapter};
function Screen() {
  const [client, setClient] = React.useState<any>(null);
  const [snapshot, setSnapshot] = React.useState<any>(null);
  const [state, setState] = React.useState('connecting');
  React.useEffect(() => {
    let live = true;
    let remove: (() => void) | undefined;
    let connected: any;
    connectRecordClient((window as any).apparent.record).then(value => {
      if (!live) { value.detach(); return; }
      connected = value;
      remove = value.observe((next: any) => setSnapshot(next));
      setClient(value); setState('connected');
    }).catch(() => { if (live) setState('connect-unavailable'); });
    return () => { live = false; remove?.(); connected?.detach(); };
  }, []);
  const run = async (kind: string) => {
    if (!client || !snapshot) { setState('not-ready'); return; }
    const expected = { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision };
    let command: any;
    if (kind === 'refresh' || kind === 'reconcile') command = { type: 'refresh', requestId: kind + '-1', expected };
    else if (kind === 'stale') command = { type: 'refresh', requestId: 'stale-1', expected: { ...expected, revision: Math.max(0, expected.revision - 1) } };
    else if (kind === 'uncertain-stop') command = { type: 'stop-capture', requestId: 'uncertain-stop-1', captureId: 'capture-active', expected };
    else if (kind === 'invalid') command = { type: 'refresh', requestId: 'invalid-1', expected, ownerId: 'foreign-owner' };
    try {
      const result = await client.execute(command);
      setState(kind + ':' + result.status + ':' + result.snapshot.revision + ':' + result.snapshot.captures[0]?.phase);
    } catch { setState(kind + ':rejected-locally'); }
  };
  const close = () => (window as any).close();
  React.useEffect(() => (window as any).apparent.host.observeCloseRefusal((reason: string) => setState('close:' + reason)), []);
  return <section><h2>Record bridge test</h2><output data-testid="state">{state}</output>
    <output data-testid="snapshot">{snapshot ? snapshot.revision + ':' + snapshot.connection + ':' + (snapshot.captures[0]?.phase ?? 'no-capture') : 'none'}</output>
    <button onClick={() => run('refresh')}>Refresh</button><button onClick={() => run('stale')}>Stale command</button>
    <button onClick={() => run('invalid')}>Invalid command</button><button onClick={() => run('uncertain-stop')}>Lose stop reply</button>
    <button onClick={() => run('reconcile')}>Reconcile provider</button><button onClick={close}>Close window</button></section>;
}
export default { id: 'record-test', Screen, Settings: Screen };
`);
  const entry = join(source, 'index.tsx');
  const outfile = join(output, 'index.js');
  const result = await build({ bundle: true, format: 'esm', platform: 'browser', target: 'es2022', jsx: 'automatic',
    nodePaths: [join(root, 'node_modules')], define: { 'process.env.NODE_ENV': '"production"' }, legalComments: 'inline',
    entryPoints: [entry], outfile, external, metafile: true });
  for (const item of Object.values(result.metafile!.outputs)) {
    assert(item.imports.every(dependency => external.includes(dependency.path)), 'Unbundled Record stream dependency');
  }
  const manifest = parseStreamManifest({ id: 'record-test', label: 'Record Test', version: '1.0.0',
    schemaVersion: 1, shellApi: APP_SHELL_API, reactVersion: REACT_VERSION,
    entry: 'index.js', sha256: digest(await readRegular(outfile)) });
  await writeFile(join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
  return manifest;
}

test('packaged Electron routes public RecordClient through trusted main, preload and renderer',
  { timeout: 180_000 }, async t => {
  assert(process.platform === 'linux' || process.platform === 'darwin', 'Supported native hosts are Linux and macOS');
  const directory = await realpath(await mkdtemp(join(tmpdir(), 'record-bridge-electron-')));
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  const sandboxes: Array<{ path: string; bytes: Buffer; mode: number }> = [];
  t.after(async () => {
    try { await stopElectron(application); }
    finally {
      for (const sandbox of sandboxes) {
        await rm(sandbox.path);
        await writeFile(sandbox.path, sandbox.bytes, { mode: sandbox.mode });
        await chmod(sandbox.path, sandbox.mode);
      }
      await rm(directory, { recursive: true, force: true });
    }
  });

  const bindingSource = join(directory, 'record-main-binding.ts');
  await writeFile(bindingSource, `
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createRecordFixture } from '@apparent/record/fixtures';
export async function createRecordHostServices({profilePath}: {profilePath:string}) {
  await mkdir(profilePath, { recursive: true });
  const fixture = createRecordFixture('unsubscribe-during-recording');
  let uncertain = false;
  let detachCalls = 0;
  const commandLog = join(profilePath, 'provider-commands.json');
  const record = {
    observe: fixture.client.observe.bind(fixture.client),
    async execute(command: any) {
      let reply: any;
      try {
        reply = await fixture.client.execute(command);
        if (command.type === 'refresh' && reply.status === 'accepted') {
          const current = fixture.driver.current();
          fixture.driver.publish({ ...current, revision: current.revision + 1,
            captures: current.captures.map(capture => uncertain && capture.captureId === 'capture-active' ? { ...capture, phase: 'cancelled' } : capture) });
          reply = { ...reply, snapshot: fixture.driver.current() };
          uncertain = false;
        }
        if (command.requestId === 'uncertain-stop-1') {
          uncertain = true;
          throw new Error('Synthetic provider reply lost after dispatch');
        }
        return reply;
      } finally {
        await writeFile(commandLog, JSON.stringify(fixture.driver.commands()));
      }
    },
    detach() { detachCalls++; fixture.client.detach(); },
  };
  return {
    record,
    async canClose() { return !uncertain && !fixture.driver.captureContinues(); },
    async close() { await writeFile(join(profilePath, 'host-closed'), JSON.stringify({ detachCalls })); },
  };
}
`);

  const shell = join(directory, 'shell');
  await buildShell(shell, undefined, { module: bindingSource, resolvePaths: [join(root, 'node_modules')] });
  const stream = join(directory, 'record-stream');
  await buildSyntheticRecordStream(join(directory, 'record-source'), stream);
  const app = await assemble(shell, [stream], join(directory, 'assembled'));
  const bundle = await packageApplication(app, join(directory, 'packages'));
  const profile = join(directory, 'profile');
  await mkdir(profile, { recursive: true });
  const sandboxHelper = process.env.APPARENT_FOUNDATION_SANDBOX_HELPER;
  if (process.platform === 'linux' && sandboxHelper) {
    const metadata = await stat(sandboxHelper);
    assert(metadata.isFile() && metadata.uid === 0 && (metadata.mode & 0o4000) && !(metadata.mode & 0o022), 'Sandbox helper must be root-owned, setuid and not writable by others');
    const target = join(bundle, 'chrome-sandbox');
    sandboxes.push({ path: target, bytes: await readFile(target), mode: (await stat(target)).mode & 0o777 });
    await rm(target);
    await symlink(sandboxHelper, target);
  }
  const appPayload = process.platform === 'darwin' ? join(bundle, 'Contents/Resources/app') : join(bundle, 'resources/app');
  await verifyAssembly(appPayload);
  const executablePath = process.platform === 'darwin' ? join(bundle, 'Contents/MacOS/apparent-foundation') : join(bundle, 'apparent-foundation');
  application = await electron.launch({ executablePath, args: [], env: { ...process.env, APPARENT_FOUNDATION_PROFILE: profile } });
  const page = application.windows()[0] ?? await application.waitForEvent('window', { timeout: 15_000 });
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await expect(page.getByRole('heading', { name: 'Record bridge test' })).toBeVisible();
  const state = page.getByTestId('state');
  const snapshot = page.getByTestId('snapshot');
  await expect(snapshot).toHaveText('1:connected:recording');

  assert.deepEqual(await page.evaluate(() => ({
    require: typeof (window as any).require, process: typeof (window as any).process,
    ipcRenderer: typeof (window as any).ipcRenderer, fs: typeof (window as any).fs,
    bridge: Object.keys((window as any).apparent.record).sort(),
  })), { require: 'undefined', process: 'undefined', ipcRenderer: 'undefined', fs: 'undefined', bridge: ['execute', 'observe', 'read'] });
  assert.equal(await application.evaluate(({ BrowserWindow }) => {
    const pref = (BrowserWindow.getAllWindows()[0].webContents as any).getLastWebPreferences();
    return pref.contextIsolation && pref.sandbox && !pref.nodeIntegration && pref.webSecurity && !pref.nodeIntegrationInSubFrames;
  }), true);

  await page.getByRole('button', { name: 'Refresh' }).click();
  await expect(state).toHaveText('refresh:accepted:2:recording');
  await expect(snapshot).toHaveText('2:connected:recording');
  await page.getByRole('button', { name: 'Stale command' }).click();
  await expect(state).toContainText('stale:refused:2:recording');
  await page.getByRole('button', { name: 'Invalid command' }).click();
  await expect(state).toHaveText('invalid:rejected-locally');
  const commandsPath = join(profile, 'provider-commands.json');
  const beforeUncertain = JSON.parse(await readFile(commandsPath, 'utf8'));
  assert.equal(beforeUncertain.some((command: any) => command.requestId === 'invalid-1'), false, 'Malformed renderer command never reaches the owner');

  await page.getByRole('button', { name: 'Lose stop reply' }).click();
  await expect(state).toHaveText('uncertain-stop:outcome-unknown:2:recording');
  const afterUncertain = JSON.parse(await readFile(commandsPath, 'utf8'));
  assert.equal(afterUncertain.filter((command: any) => command.requestId === 'uncertain-stop-1').length, 1, 'Lost reply is never retried');

  const secondaryWindow = application.waitForEvent('window');
  const preload = join(appPayload, 'preload.cjs');
  const entryUrl = pathToFileURL(join(appPayload, 'index.html')).href;
  await application.evaluate(({ BrowserWindow }, input) => {
    const second = new BrowserWindow({ show: true, webPreferences: { preload: input.preload,
      nodeIntegration: false, contextIsolation: true, sandbox: true, webSecurity: true } });
    void second.loadURL(input.entryUrl);
  }, { preload, entryUrl });
  const unauthorized = await secondaryWindow;
  await expect(unauthorized.getByRole('heading', { name: 'Record bridge test' })).toBeVisible();
  assert.equal(await unauthorized.evaluate(async () => {
    try { await (window as any).apparent.record.read(); return false; } catch { return true; }
  }), true, 'A second BrowserWindow cannot read the registered window’s provider');
  assert.equal(await page.evaluate(() => {
    const frame = document.createElement('iframe'); document.body.append(frame);
    return typeof (frame.contentWindow as any).apparent?.record;
  }), 'undefined', 'A subframe receives no Record preload capability');
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(window => window !== BrowserWindow.getAllWindows()[0])?.close());

  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].close());
  await expect(state).toHaveText('close:ownership-unresolved');
  assert.equal(await readFile(join(profile, 'host-closed')).then(() => true, () => false), false, 'Unknown ownership blocks host cleanup and window close');

  await page.getByRole('button', { name: 'Reconcile provider' }).click();
  await expect(state).toHaveText('reconcile:accepted:3:cancelled');
  await expect(snapshot).toHaveText('3:connected:cancelled');
  const finishedProcess = application.process();
  const finished = new Promise<void>(resolve => finishedProcess.once('exit', () => resolve()));
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].close());
  let finishTimer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([finished, new Promise<never>((_, reject) => { finishTimer = setTimeout(() => reject(new Error('Record host shutdown did not finish')), 10_000); })]);
  } finally { clearTimeout(finishTimer); }
  application = undefined;
  const cleanup = JSON.parse(await readFile(join(profile, 'host-closed'), 'utf8'));
  assert.deepEqual(cleanup, { detachCalls: 0 }, 'Shutdown cleanup does not detach the caller-owned RecordClient');
  const finalCommands = JSON.parse(await readFile(commandsPath, 'utf8'));
  assert.equal(finalCommands.filter((command: any) => command.type === 'cancel-capture').length, 0, 'Shutdown does not synthesize capture cancellation');
  assert.equal(finalCommands.filter((command: any) => command.type === 'stop-capture').length, 1, 'Only the explicit test action reached the provider');
  assert.deepEqual(errors, []);
  t.diagnostic('Packaged Record path verified: public fixture read/observe, monotonic provider update, accepted/stale/local-invalid commands, lost-reply unknown without retry, foreign-window denial, subframe isolation, close refusal during unknown ownership, and cleanup without client detach or synthetic cancellation.');
});

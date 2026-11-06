import assert from 'node:assert/strict';
import { test } from 'node:test';
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { _electron as electron, expect } from '@playwright/test';
import { buildShell, root } from '../scripts/build.ts';
import { assemble, verifyAssembly } from '../scripts/assemble.ts';
import { packageApplication } from '../scripts/package.ts';
const { stopElectron } = createRequire(import.meta.url)('../../../enablement/native/owned-processes.ts') as {
  stopElectron(application: Awaited<ReturnType<typeof electron.launch>> | undefined): Promise<void>;
};

test('packaged Record provider retains unresolved runtime and closes only after authoritative reconciliation',
  { timeout: 180_000 }, async t => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), 'record-provider-electron-')));
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  let sandbox: { path: string; bytes: Buffer; mode: number } | undefined;
  t.after(async () => {
    try { await stopElectron(application); }
    finally {
      if (sandbox) { await rm(sandbox.path); await writeFile(sandbox.path, sandbox.bytes, { mode: sandbox.mode }); await chmod(sandbox.path, sandbox.mode); }
      await rm(directory, { recursive: true, force: true });
    }
  });
  const binding = join(directory, 'binding.ts');
  // Real public Record provider and production host lifecycle. Only Engine/native
  // producers are synthetic: this proves transport/ownership, not OS effects.
  await writeFile(binding, `
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { CaptureClient, preflightCapturePublication } from '@apparent/engine-sdk/capture';
import { createRecordPublicationOutput } from '@apparent/record/publication';
import { createRecordProviderHost } from ${JSON.stringify(join(root, 'src/record/provider-host.ts'))};
export async function createRecordHostServices({profilePath}) {
  let outputDispatches = 0;
  const sdk = new CaptureClient({async request() { outputDispatches++; throw new Error('No native operation admitted'); }, async close() {return 'already-exited';}});
  const output = createRecordPublicationOutput(sdk,join(profilePath,'publication-output'),preflightCapturePublication);
  await writeFile(join(profilePath,'public-output.json'),JSON.stringify({facts:await output.captureFacts.readFacts(),handoff:await output.resolveHandoff({captureId:'unknown-request',artifactId:'unknown-bundle'}),outputDispatches}));
  await sdk.close();
  let reconciliation = 'pending';
  const counts = { dispatch: 0, dispose: 0, engine: 0, native: 0 };
  const listeners = new Set();
  const state = { phase: 'idle', readiness: {ready: false, reason: 'Synthetic runtime not reconciled'} };
  const runtime = {
    state, startup: {state: 'ready', knownRequestIds: []},
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    async dispatch() { counts.dispatch++; reconciliation = 'complete'; for (const listener of listeners) listener(state); return state; },
    async reconcileStartup() { return {state: 'ready', knownRequestIds: []}; },
    unresolvedOwnership() { return {requestIds: [], storageUnavailable: false, reconciliation}; },
    dispose() { counts.dispose++; listeners.clear(); return this.unresolvedOwnership(); },
  };
  return createRecordProviderHost({runtime, nativeBindings: {
    native: {async readFacts() { return { version: 1, providerEpoch: 'synthetic-native', revision: 1,
      readiness: [], shortcut: {effective: null, label: null, registration: 'unknown', externallyManaged: false, draft: null} }; }},
    penConfig: {async read() { return {persisted: null, effective: null}; }, async writeApplyAndReadBack() { return {status: 'unknown', effective: null}; }},
    clipboard: {async copyLocal() { return {status: 'refused', reason: 'not-saved'}; }},
    async close() { counts.native++; await writeFile(join(profilePath, 'closed.json'), JSON.stringify(counts)); },
  }, async closeEngine() { counts.engine++; }});
}
`);
  const shell = join(directory, 'shell');
  await buildShell(shell, undefined, { module: binding });
  const assembled = await assemble(shell, [], join(directory, 'app'));
  const bundle = await packageApplication(assembled, join(directory, 'packages'));
  const helper = process.env.APPARENT_FOUNDATION_SANDBOX_HELPER;
  if (process.platform === 'linux' && helper) {
    const metadata = await stat(helper);
    assert(metadata.isFile() && metadata.uid === 0 && (metadata.mode & 0o4000) && !(metadata.mode & 0o022));
    const target = join(bundle, 'chrome-sandbox');
    sandbox = { path: target, bytes: await readFile(target), mode: (await stat(target)).mode & 0o777 };
    await rm(target); await symlink(helper, target);
  }
  const payload = process.platform === 'darwin' ? join(bundle, 'Contents/Resources/app') : join(bundle, 'resources/app');
  await verifyAssembly(payload);
  const executablePath = process.platform === 'darwin' ? join(bundle, 'Contents/MacOS/apparent-foundation') : join(bundle, 'apparent-foundation');
  const profile = join(directory, 'profile'); await mkdir(profile, { mode: 0o700 });
  application = await electron.launch({ executablePath, args: [], env: { ...process.env, APPARENT_FOUNDATION_PROFILE: profile } });
  const page = application.windows()[0] ?? await application.waitForEvent('window');
  await expect(page.getByRole('heading', { name: 'Apparent Foundation' })).toBeVisible();
  await page.evaluate(() => { (window as any).apparent.host.observeCloseRefusal((reason: string) => { document.body.dataset.closeReason = reason; }); });
  const initial = await page.evaluate(() => (window as any).apparent.record.read());
  assert.equal(initial.connection, 'connected');
  const publicOutput = JSON.parse(await readFile(join(profile, 'public-output.json'), 'utf8'));
  assert.deepEqual(publicOutput.facts.captures, []);
  assert.deepEqual(publicOutput.facts.nextShortcutAdmission, { state: 'unavailable' });
  assert.equal(publicOutput.handoff, null);
  assert.equal(publicOutput.outputDispatches, 0);
  await application.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0].close(); });
  await expect(page.locator('body')).toHaveAttribute('data-close-reason', 'ownership-unresolved');
  assert.equal(await readFile(join(profile, 'closed.json')).then(() => true, () => false), false);
  const result = await page.evaluate(async () => {
    const port = (window as any).apparent.record;
    const snapshot = await port.read();
    return port.execute({ type: 'refresh', requestId: 'reconcile-runtime', expected: {providerEpoch: snapshot.providerEpoch, revision: snapshot.revision} });
  });
  assert.equal(result.status, 'accepted');
  const exited = application.waitForEvent('close');
  await application.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0].close(); });
  await exited; application = undefined;
  assert.deepEqual(JSON.parse(await readFile(join(profile, 'closed.json'), 'utf8')), {dispatch: 1, dispose: 1, engine: 1, native: 1});
});

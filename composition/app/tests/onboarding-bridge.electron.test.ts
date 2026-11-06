import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { _electron as electron, expect } from '@playwright/test';
import { build } from 'esbuild';
import { createRequire } from 'node:module';
const { stopElectron } = createRequire(import.meta.url)('../../../enablement/native/owned-processes.ts') as {
  stopElectron(application: Awaited<ReturnType<typeof electron.launch>> | undefined): Promise<void>;
};
import { buildShell, root } from '../scripts/build.ts';
import { assemble, verifyAssembly } from '../scripts/assemble.ts';
import { packageApplication } from '../scripts/package.ts';
import { digest, readRegular } from '../scripts/artifacts.ts';
import { buildSyntheticProgressStream } from './synthetic-stream.ts';

const CONFIG_HOST_SHA256 = process.platform === 'darwin'
  ? '503a7d897790c8b25ed6b7e53ee0f0fdee4573bcb85999b21d9967e112642e6c'
  : 'b1a89cae8351770eb42bc1876ab9718fe336e618ab238a387c00c48f11eef5d5';
const helperInput = process.env.APPARENT_ONBOARDING_CONFIG_HOST;

test('packaged Electron routes the compiled Onboarding port through real main, preload and native config host',
  { timeout: 180_000, skip: !helperInput && 'Set APPARENT_ONBOARDING_CONFIG_HOST to the attested platform helper for the native integration lane.' }, async t => {
  assert(process.platform === 'linux' || process.platform === 'darwin', 'Supported native hosts are Linux and macOS');
  const helperPath = resolve(helperInput!);
  const helperStat = await stat(helperPath);
  assert(helperStat.isFile() && (helperStat.mode & 0o111) && !(helperStat.mode & 0o022));
  assert.equal(digest(await readRegular(helperPath)), CONFIG_HOST_SHA256, 'Unexpected admitted config helper');

  const directory = await realpath(await mkdtemp(join(tmpdir(), 'onboarding-progress-electron-')));
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  let succeeded = false;
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

  const artifacts = join(directory, 'published-packages');
  await mkdir(artifacts);
  const packageArtifacts = [
    { path: resolve(root, '../../shared/contracts'), name: 'apparent-app-contracts-0.1.0.tgz' },
    { path: resolve(root, '../../onboarding'), name: 'apparent-onboarding-0.1.0.tgz' },
    { path: resolve(root, '../config'), name: 'apparent-config-host-0.1.0.tgz' },
  ];
  for (const item of packageArtifacts) execFileSync('npm', ['pack', '--pack-destination', artifacts], { cwd: item.path, stdio: 'pipe' });
  const packages = join(directory, 'owner-packages');
  await mkdir(packages);
  execFileSync('npm', ['install', '--prefix', packages, '--no-save', '--package-lock=false', ...packageArtifacts.map(item => join(artifacts, item.name))], { stdio: 'pipe' });

  const bindingSource = join(directory, 'onboarding-main-binding.ts');
  const hostServices = JSON.stringify(join(root, 'src/host-services.ts'));
  await writeFile(bindingSource, `
import { createConfigBackend } from '@apparent/config-host';
import { writeFile } from 'node:fs/promises';
import { adoptOnboardingProgress, onboardingProgressCodec } from '@apparent/onboarding';
import { bindOwnerProgress } from ${hostServices};
const helperSha256 = ONBOARDING_CONFIG_HOST_SHA256;
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
export async function createOnboardingProgressStore({configPath, configHostPath}: {configPath:string; configHostPath:string}) {
  const backend = await createConfigBackend({ executable: configHostPath, executableSha256: helperSha256, configPath, ownerId: 'onboarding' });
  const retired = await backend.retireOnboardingLegacyWriter();
  if (retired.status !== 'retired') { await backend.close(); throw new Error('Synthetic legacy writer retirement failed'); }
  const adoption = await adoptOnboardingProgress(backend);
  if (adoption.status !== 'ready') { await backend.close(); throw new Error('Synthetic Onboarding adoption failed'); }
  const real = bindOwnerProgress(backend, 'onboarding', onboardingProgressCodec);
  let readCount = 0; let writeCount = 0;
  let releaseReply: (() => void) | undefined;
  const store = {
    async read() { if (++readCount === 2) throw new Error('Injected synthetic read failure'); return real.read(); },
    async compareAndSwap(expected: any, value: any) {
      const call = ++writeCount;
      if (call === 4) return { status: 'saved', revision: Number.MAX_SAFE_INTEGER + 1 } as any;
      const result = await real.compareAndSwap(expected, value);
      if (call === 3) throw new Error('Injected synthetic lost provider reply after native commit');
      if (call === 5) await new Promise<void>(resolve => { releaseReply = resolve; });
      return result;
    },
  };
  return { store, close: async () => { releaseReply?.(); await backend.close(); await delay(200); await writeFile(configPath + '.closed', 'closed'); } };
}
`);
  const shell = join(directory, 'shell');
  await buildShell(shell, { module: bindingSource, configHost: { path: helperPath, sha256: CONFIG_HOST_SHA256 },
    ownerAssets: join(packages, 'node_modules/@apparent/onboarding/dist/assets'),
    resolvePaths: [join(packages, 'node_modules')] });
  const stream = join(directory, 'onboarding-stream');
  await buildSyntheticProgressStream(join(directory, 'owner-renderer-source'), stream);
  const assembled = await assemble(shell, [stream], join(directory, 'app'));
  const bundle = await packageApplication(assembled, join(directory, 'packages'));

  const profile = join(directory, 'synthetic-profile');
  await mkdir(profile, { recursive: true });
  const legacy = { seen: true, deferred: true, resume: 'permissions', granted: ['microphone'] };
  const configPath = join(profile, 'config.json');
  await writeFile(configPath, JSON.stringify({ onboarding: legacy, secret: 'synthetic-only',
    streamState: { agent: { revision: 7, value: { keep: true } } } }), { mode: 0o600 });
  const sandboxHelper = process.env.APPARENT_FOUNDATION_SANDBOX_HELPER;
  const sandboxed = new Set<string>();
  const prepareSandbox = async (appBundle: string) => {
    if (process.platform !== 'linux' || !sandboxHelper || sandboxed.has(appBundle)) return;
    const metadata = await stat(sandboxHelper);
    assert(metadata.isFile() && metadata.uid === 0 && (metadata.mode & 0o4000) && !(metadata.mode & 0o022), 'Invalid Chromium sandbox helper');
    const target = join(appBundle, 'chrome-sandbox');
    sandboxes.push({ path: target, bytes: await readFile(target), mode: (await stat(target)).mode & 0o777 });
    await rm(target); await symlink(sandboxHelper, target); sandboxed.add(appBundle);
  };
  await prepareSandbox(bundle);
  const appPayload = process.platform === 'darwin' ? join(bundle, 'Contents/Resources/app') : join(bundle, 'resources/app');
  await verifyAssembly(appPayload);
  const executablePath = process.platform === 'darwin' ? join(bundle, 'Contents/MacOS/apparent-foundation') : join(bundle, 'apparent-foundation');
  application = await electron.launch({ executablePath, args: ['--enable-logging=stderr'], env: { ...process.env, APPARENT_FOUNDATION_PROFILE: profile } });
  let page;
  try { page = application.windows()[0] ?? await application.waitForEvent('window', { timeout: 15_000 }); }
  catch (error) {
    try { t.diagnostic(JSON.stringify(await application.evaluate(({ app, BrowserWindow }) => ({ ready: app.isReady(), windows: BrowserWindow.getAllWindows().length })))); }
    catch (diagnostic) { t.diagnostic(String(diagnostic)); }
    throw error;
  }
  await expect(page.getByRole('heading', { name: 'Onboarding bridge test' })).toBeVisible();
  const primaryWindowId = await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].id);
  const state = page.getByTestId('bridge-state');
  const revision = page.getByTestId('bridge-revision');
  await page.getByRole('button', { name: 'Read progress' }).click();
  await expect(state).toContainText('read:1:complete');
  await page.getByRole('button', { name: 'Extra arguments' }).click();
  await expect(state).toHaveText('extra-rejected-locally');
  await page.getByRole('button', { name: 'Malformed progress' }).click();
  await expect(state).toHaveText('invalid-rejected-locally');
  await page.getByRole('button', { name: 'Save progress' }).click();
  await expect(state).toHaveText('saved:2');
  await page.getByRole('button', { name: 'Save stale' }).click();
  await expect(state).toHaveText('conflict:2');
  await page.getByRole('button', { name: 'Fail read' }).click();
  await expect(state).toHaveText('read-unavailable');
  await page.getByRole('button', { name: 'Save uncertain' }).click();
  await expect(state).toHaveText('unknown');
  await page.getByRole('button', { name: 'Read progress' }).click();
  await expect(state).toContainText('read:3:complete');
  await page.getByRole('button', { name: 'Malformed result' }).click();
  await expect(state).toHaveText('unknown');
  await page.getByRole('button', { name: 'Read progress' }).click();
  await expect(state).toContainText('read:3:complete');
  assert.equal(await page.evaluate(() => JSON.stringify({ exposed: Object.keys(window.apparent), methods: Object.keys(window.apparent.onboardingProgress),
    require: typeof (window as any).require, process: typeof (window as any).process, ipcRenderer: typeof (window as any).ipcRenderer,
    fs: typeof (window as any).fs })), JSON.stringify({ exposed: ['onboardingProgress'], methods: ['read', 'compareAndSwap'],
    require: 'undefined', process: 'undefined', ipcRenderer: 'undefined', fs: 'undefined' }));
  assert.equal(await application.evaluate(({ BrowserWindow }) => {
    const pref = (BrowserWindow.getAllWindows()[0].webContents as any).getLastWebPreferences();
    return pref.contextIsolation && pref.sandbox && !pref.nodeIntegration && pref.webSecurity && !pref.nodeIntegrationInSubFrames;
  }), true);
  const iframeFacade = await page.evaluate(() => {
    const iframe = document.createElement('iframe'); document.body.append(iframe);
    return typeof (iframe.contentWindow as any).apparent;
  });
  assert.equal(iframeFacade, 'undefined', 'Subframes must not receive the preload capability');
  // A second BrowserWindow receives the same real preload but is not registered.
  const unauthorizedEvent = application.waitForEvent('window');
  await application.evaluate(({ BrowserWindow }, input) => {
    const second = new BrowserWindow({ show: true, webPreferences: { preload: input.preload,
      nodeIntegration: false, contextIsolation: true, sandbox: true, webSecurity: true } });
    (globalThis as any).__onboardingUnauthorizedWindow = second;
    void second.loadURL(input.entryUrl);
  }, { preload: join(appPayload, 'preload.cjs'), entryUrl: pathToFileURL(join(appPayload, 'index.html')).href });
  const unauthorizedPage = await unauthorizedEvent;
  await expect(unauthorizedPage.getByRole('heading', { name: 'Onboarding bridge test' })).toBeVisible();
  await unauthorizedPage.getByRole('button', { name: 'Attempt direct CAS' }).click();
  await expect(unauthorizedPage.getByTestId('bridge-state')).toHaveText('unknown');

  // Commit natively, then destroy its sender while the reply is held. The second
  // window keeps the process/backend alive so we can verify one durable write.
  await page.getByRole('button', { name: 'Save delayed' }).click();
  await expect(state).toHaveText('saving-delayed');
  await expect.poll(async () => JSON.parse(await readFile(configPath, 'utf8')).streamState.onboarding.revision).toBe(4);
  assert.equal(await revision.textContent(), '3');
  await application.evaluate(async ({ BrowserWindow }, id) => {
    const owned = BrowserWindow.fromId(id)!;
    const closed = new Promise<void>(resolve => owned.once('closed', () => resolve()));
    owned.close();
    await closed;
  }, primaryWindowId);
  // Public registration succeeds only after disposal removed the old handler.
  await application.evaluate(({ ipcMain }) => {
    ipcMain.handle('apparent:onboarding-progress:read', async () => ({ revision: 0, value: { marker: 'replacement' } }));
  });
  await new Promise(resolve => setTimeout(resolve, 1_200));
  const persisted = JSON.parse(await readFile(configPath, 'utf8'));
  assert.equal(persisted.secret, 'synthetic-only');
  assert.deepEqual(persisted.streamState.agent, { revision: 7, value: { keep: true } });
  assert.deepEqual(persisted.onboarding, legacy, 'Legacy data remains preserved');
  assert.equal(persisted.streamState.onboarding.revision, 4, 'No IPC retry duplicated the delayed write');
  assert.equal(await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length), 1);
  assert.deepEqual(await unauthorizedPage.evaluate(() => window.apparent.onboardingProgress.read()),
    { revision: 0, value: { marker: 'replacement' } }, 'Old disposal must leave a later public registration intact');
  await stopElectron(application);
  application = undefined;
  // With no second window, quitting must await the privileged host close.
  await rm(configPath + '.closed', { force: true });
  application = await electron.launch({ executablePath, args: [], env: { ...process.env, APPARENT_FOUNDATION_PROFILE: profile } });
  const reopened = application.windows()[0] ?? await application.waitForEvent('window');
  await expect(reopened.getByRole('heading', { name: 'Onboarding bridge test' })).toBeVisible();
  const finishedProcess = application.process();
  const finished = new Promise<void>(resolve => finishedProcess.once('exit', () => resolve()));
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].close());
  let finishTimer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([finished, new Promise<never>((_, reject) => { finishTimer = setTimeout(() => reject(new Error('Host shutdown did not finish')), 10_000); })]);
    assert.equal(await readFile(configPath + '.closed', 'utf8'), 'closed', 'Application quit before host cleanup completed');
  } finally { clearTimeout(finishTimer); }
  application = undefined;
  succeeded = true;
  t.diagnostic('Verified owner read/CAS through native config, conflict, uncertainty/no retry, malformed inputs/results, unauthorized window, subframe isolation, and pending-write revocation in packaged Electron.');
});

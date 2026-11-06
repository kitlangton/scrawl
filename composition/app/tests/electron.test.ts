import assert from 'node:assert/strict';
import { test } from 'node:test';
import { chmod, mkdir, mkdtemp, rm, stat, symlink, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { createRequire } from 'node:module';
import { _electron as electron, expect } from '@playwright/test';
import { buildShell, root } from '../scripts/build.ts';
import { assemble, verifyAssembly } from '../scripts/assemble.ts';
import { packageApplication } from '../scripts/package.ts';
import { buildSyntheticStream } from './synthetic-stream.ts';
import { digest } from '../scripts/artifacts.ts';

test('packaged Electron composes screens and owner settings, preserves preferences and isolates renderer', { timeout: 120_000 }, async t => {
  const directory = await mkdtemp(join(tmpdir(), 'foundation-electron-'));
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  const sandboxes: Array<{ path: string; bytes: Buffer; mode: number }> = [];
  const sandboxedBundles = new Set<string>();
  t.after(async () => {
    try { await application?.close(); }
    finally {
      // The CI test may consume the exact bundle that will be shipped. Restore
      // the original sandbox binary rather than shipping a runner-local symlink.
      for (const sandbox of sandboxes) {
        await rm(sandbox.path);
        await writeFile(sandbox.path, sandbox.bytes, { mode: sandbox.mode });
        await chmod(sandbox.path, sandbox.mode);
      }
      await rm(directory, { recursive: true, force: true });
    }
  });
  const shell = join(directory, 'shell');
  await buildShell(shell);
  // Restricted hosts may supply an already-installed, root-owned SUID helper.
  // Never disable Chromium sandboxing or change system permissions for a test.
  const helper = process.env.APPARENT_FOUNDATION_SANDBOX_HELPER;
  const prepareSandboxHelper = async (bundle: string) => {
    if (!helper || process.platform !== 'linux' || sandboxedBundles.has(bundle)) return;
    const metadata = await stat(helper);
    assert(metadata.isFile() && metadata.uid === 0 && (metadata.mode & 0o4000) && !(metadata.mode & 0o022),
      'Sandbox helper must be root-owned, setuid and not writable by other users');
    const path = join(bundle, 'chrome-sandbox');
    sandboxes.push({ path, bytes: await readFile(path), mode: (await stat(path)).mode & 0o777 });
    await rm(path);
    await symlink(helper, path);
    sandboxedBundles.add(bundle);
  };
  const launch = async (bundle: string, profile: string) => {
    await prepareSandboxHelper(bundle);
    await verifyAssembly(process.platform === 'darwin'
      ? join(bundle, 'Contents/Resources/app') : join(bundle, 'resources/app'));
    const executablePath = process.platform === 'darwin'
      ? join(bundle, 'Contents/MacOS/apparent-foundation') : join(bundle, 'apparent-foundation');
    return electron.launch({ executablePath, args: [], env: { ...process.env,
      APPARENT_FOUNDATION_PROFILE: profile } });
  };
  const exactBundle = process.env.APPARENT_FOUNDATION_PACKAGE;
  if (exactBundle) {
    application = await launch(exactBundle, join(directory, 'empty-profile'));
    const page = await application.firstWindow();
    await expect(page.getByText('No streams installed.')).toBeVisible();
    await application.close();
    application = undefined;
  }
  const alpha = join(directory, 'alpha');
  const beta = join(directory, 'beta');
  const alphaSource = join(directory, 'source-alpha');
  const betaSource = join(directory, 'source-beta');
  await buildSyntheticStream(alphaSource, alpha, { id: 'alpha', label: 'Alpha',
    description: 'Synthetic Alpha workflow.', setting: { kind: 'checkbox', label: 'Alpha preference' } });
  await buildSyntheticStream(betaSource, beta, { id: 'beta', label: 'Beta',
    description: 'Synthetic Beta workflow.', setting: { kind: 'select', label: 'Beta detail',
      initial: 'brief', options: ['brief', 'full'] } });
  const recordCss = createRequire(import.meta.url).resolve('@apparent/record/ui/style.css');
  const font = await readFile(join(dirname(recordCss), 'HostGrotesk-Variable.ttf'));
  await mkdir(join(alpha, 'fonts'));
  await writeFile(join(alpha, 'fonts/record.ttf'), font);
  const css = '@font-face{font-family:"Host Grotesk";src:url("./fonts/record.ttf") format("truetype")} h2{font-family:"Host Grotesk";color:rgb(19,83,121)}';
  await writeFile(join(alpha, 'style.css'), css);
  const manifest = JSON.parse(await readFile(join(alpha, 'manifest.json'), 'utf8'));
  await writeFile(join(alpha, 'manifest.json'), JSON.stringify({ ...manifest,
    assets: { 'style.css': digest(css), 'fonts/record.ttf': digest(font) } }));
  const output = await assemble(shell, [alpha, beta], join(directory, 'app'));
  const bundle = await packageApplication(output, join(directory, 'packages'));
  application = await launch(bundle, join(directory, 'profile'));
  let page = await application.firstWindow();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') t.diagnostic(message.text()); });
  try { await expect(page.getByRole('heading', { name: 'Alpha screen' })).toBeVisible(); }
  catch (error) { t.diagnostic(await page.content()); t.diagnostic(errors.join('\n')); throw error; }
  assert.deepEqual(await page.getByRole('heading', { name: 'Alpha screen' }).evaluate(async heading => {
    const fonts = await document.fonts.load('18px "Host Grotesk"', 'Alpha screen');
    return { color: getComputedStyle(heading).color, family: getComputedStyle(heading).fontFamily,
      loaded: fonts.length > 0 && fonts.every(font => font.status === 'loaded') };
  }), { color: 'rgb(19, 83, 121)', family: '"Host Grotesk"', loaded: true }, 'Packaged local stylesheet and real Record font load under the shipped CSP');
  await page.getByRole('button', { name: 'Beta', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Beta screen' })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Alpha preference' }).check();
  await expect(page.getByRole('combobox', { name: 'Beta detail' })).toHaveValue('brief');
  await page.getByRole('combobox', { name: 'Beta detail' }).selectOption('full');
  await page.getByRole('button', { name: 'Alpha', exact: true }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Alpha preference' })).toBeChecked();
  assert.deepEqual(await page.evaluate(() => ({ require: typeof (window as any).require,
    process: typeof (window as any).process })), { require: 'undefined', process: 'undefined' });
  assert.equal(await application.evaluate(({ BrowserWindow }) => {
    const preferences = (BrowserWindow.getAllWindows()[0].webContents as any).getLastWebPreferences();
    return preferences.contextIsolation && preferences.sandbox && !preferences.nodeIntegration && preferences.webSecurity;
  }), true);
  assert.equal(await page.evaluate(async () => {
    try { await fetch('https://example.com'); return false; } catch { return true; }
  }), true);
  assert.deepEqual(errors, []);
  await application.close();
  application = await launch(bundle, join(directory, 'profile'));
  page = await application.firstWindow();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Alpha preference' })).toBeChecked();
  await expect(page.getByRole('combobox', { name: 'Beta detail' })).toHaveValue('full');
  if (process.env.APPARENT_FOUNDATION_SCREENSHOT) await page.screenshot({ path: process.env.APPARENT_FOUNDATION_SCREENSHOT });
});

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, rm, stat, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { _electron as electron, expect } from '@playwright/test';
import { buildShell, buildStream, root } from '../scripts/build.ts';
import { assemble } from '../scripts/assemble.ts';
import { packageApplication } from '../scripts/package.ts';

test('packaged Electron composes screens and owner settings, preserves preferences and isolates renderer', { timeout: 120_000 }, async t => {
  const directory = await mkdtemp(join(tmpdir(), 'foundation-electron-'));
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  t.after(async () => {
    await application?.close();
    await rm(directory, { recursive: true, force: true });
  });
  const shell = join(directory, 'shell');
  const alpha = join(directory, 'alpha');
  const beta = join(directory, 'beta');
  await buildShell(shell);
  await buildStream(join(root, 'fixtures/alpha'), alpha);
  await buildStream(join(root, 'fixtures/beta'), beta);
  const output = await assemble(shell, [alpha, beta], join(directory, 'app'));
  const bundle = await packageApplication(output, join(directory, 'packages'));
  // Restricted hosts may supply an already-installed, root-owned SUID helper.
  // Never disable Chromium sandboxing or change system permissions for a test.
  const helper = process.env.APPARENT_FOUNDATION_SANDBOX_HELPER;
  if (helper && process.platform === 'linux') {
    const metadata = await stat(helper);
    assert(metadata.isFile() && metadata.uid === 0 && (metadata.mode & 0o4000) && !(metadata.mode & 0o022),
      'Sandbox helper must be root-owned, setuid and not writable by other users');
    await rm(join(bundle, 'chrome-sandbox'));
    await symlink(helper, join(bundle, 'chrome-sandbox'));
  }
  const executablePath = process.platform === 'darwin'
    ? join(bundle, 'Contents/MacOS/apparent-foundation') : join(bundle, 'apparent-foundation');
  const launch = () => electron.launch({ executablePath, args: [], env: { ...process.env,
    APPARENT_FOUNDATION_PROFILE: join(directory, 'profile') } });
  application = await launch();
  let page = await application.firstWindow();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') t.diagnostic(message.text()); });
  try { await expect(page.getByRole('heading', { name: 'Alpha screen' })).toBeVisible(); }
  catch (error) { t.diagnostic(await page.content()); t.diagnostic(errors.join('\n')); throw error; }
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
  application = await launch();
  page = await application.firstWindow();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Alpha preference' })).toBeChecked();
  await expect(page.getByRole('combobox', { name: 'Beta detail' })).toHaveValue('full');
  if (process.env.APPARENT_FOUNDATION_SCREENSHOT) await page.screenshot({ path: process.env.APPARENT_FOUNDATION_SCREENSHOT });
});

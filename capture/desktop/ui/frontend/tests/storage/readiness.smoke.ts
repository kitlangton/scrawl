import { test } from 'node:test';
import assert from 'node:assert/strict';
import { _electron, chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

// Copy the shipped host/preloads/assets, then remove just its React entry module.
// No replacement main-process or renderer implementation stands in for startup.
test('missing Settings renderer stays hidden and exits for native fallback', { timeout: 25000 }, async t => {
  const frontend = fileURLToPath(new URL('../..', import.meta.url));
  const fixture = await fs.mkdtemp(path.join(os.tmpdir(), 'ap-ready-'));
  let app: Awaited<ReturnType<typeof _electron.launch>> | undefined;
  let exited = false;
  t.after(async () => {
    if (app && !exited) await app.close();
    await fs.rm(fixture, { recursive: true, force: true });
  });
  await fs.cp(path.join(frontend, 'electron'), path.join(fixture, 'electron'), { recursive: true });
  await fs.cp(path.join(frontend, 'dist'), path.join(fixture, 'dist'), { recursive: true });
  await fs.symlink(path.join(frontend, 'node_modules'), path.join(fixture, 'node_modules'), 'dir');
  await fs.writeFile(path.join(fixture, 'package.json'), JSON.stringify({ main: 'electron/main.cjs' }));
  const html = await fs.readFile(path.join(fixture, 'dist/settings.html'), 'utf8');
  const entry = html.match(/<script[^>]*src="([^"]+)"/)?.[1];
  assert.ok(entry);
  const module = path.resolve(fixture, 'dist', entry);
  assert.ok(module.startsWith(path.join(fixture, 'dist') + path.sep));
  await fs.rm(module);
  app = await _electron.launch({ args: [fixture, '--settings-storage'],
    env: { ...process.env, HOME: fixture, XDG_CONFIG_HOME: path.join(fixture, 'config') } });
  const output: string[] = [];
  app.process().stdout?.on('data', data => output.push(data.toString()));
  const exit = new Promise<number | null>(resolve => app!.process().once('exit', code => { exited = true; resolve(code); }));
  const page = await app.firstWindow();
  await page.waitForLoadState('load');
  assert.equal(await page.locator('#root').innerHTML(), '');
  assert.equal(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].isVisible()), false);
  assert.equal(await exit, 1, 'A missing entry must release the parent into native fallback');
  assert.equal(output.join('').includes('"action":"close"'), false, 'Failure must not masquerade as user dismissal');
});

// Hidden documents may defer animation callbacks even after React commits.
// Load the shipped renderer while modelling that scheduling constraint only.
test('mounted Settings becomes visible without hidden animation callbacks', { timeout: 25000 }, async t => {
  const frontend = fileURLToPath(new URL('../..', import.meta.url));
  const fixture = await fs.mkdtemp(path.join(os.tmpdir(), 'ap-mounted-'));
  let app: Awaited<ReturnType<typeof _electron.launch>> | undefined;
  t.after(async () => {
    if (app) await app.close();
    await fs.rm(fixture, { recursive: true, force: true });
  });
  await fs.cp(path.join(frontend, 'electron'), path.join(fixture, 'electron'), { recursive: true });
  await fs.cp(path.join(frontend, 'dist'), path.join(fixture, 'dist'), { recursive: true });
  await fs.symlink(path.join(frontend, 'node_modules'), path.join(fixture, 'node_modules'), 'dir');
  await fs.writeFile(path.join(fixture, 'package.json'), JSON.stringify({ main: 'electron/main.cjs' }));
  const html = path.join(fixture, 'dist/settings.html');
  await fs.writeFile(path.join(fixture, 'dist/paused-animation.js'),
    'window.requestAnimationFrame = () => 1; window.cancelAnimationFrame = () => {};');
  await fs.writeFile(html, (await fs.readFile(html, 'utf8')).replace('<head>', '<head><script src="./paused-animation.js"></script>'));
  app = await _electron.launch({ args: [fixture, '--settings-storage'],
    env: { ...process.env, HOME: fixture, XDG_CONFIG_HOME: path.join(fixture, 'config') } });
  const page = await app.firstWindow();
  await page.locator('.settings-shell').waitFor();
  const { expect } = await import('@playwright/test');
  await expect.poll(() => app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].isVisible())).toBe(true);
  await expect(page.getByText('CAPTURE SETTINGS', { exact: true })).toBeVisible();
});

test('parent EOF with broken stdout exits quietly instead of opening an error dialog', { timeout: 15000 }, async t => {
  const frontend = fileURLToPath(new URL('../..', import.meta.url));
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ap-eof-'));
  // Playwright's Electron launcher closes stdin, so retain a real parent pipe
  // and observe only the owned renderer through loopback CDP in this fixture.
  const executable = createRequire(import.meta.url)('electron') as string;
  // Match Playwright's Electron launcher on hosts without a configured SUID
  // helper. This fixture flag is never part of the installed launch command.
  const child = spawn(executable, ['--no-sandbox', frontend, '--settings-storage', '--settings-parent', '--remote-debugging-port=0'],
    {env:{...process.env, ELECTRON_RUN_AS_NODE:'', HOME:home, XDG_CONFIG_HOME:path.join(home,'config')}, stdio:'pipe'});
  let exited = false;
  let browser: Awaited<ReturnType<typeof chromium.connectOverCDP>> | undefined;
  const exit = new Promise<number | null>(resolve => child.once('exit', code => { exited = true; resolve(code); }));
  t.after(async () => {
    if (!exited) child.kill('SIGTERM');
    await browser?.close().catch(() => {});
    if (!exited) child.kill('SIGKILL');
    await exit;
    await fs.rm(home, {recursive:true, force:true});
  });
  const debuggerUrl = await new Promise<string>((resolve, reject) => {
    let stderr = '';
    child.stderr.on('data', chunk => {
      stderr += chunk.toString();
      const url = stderr.match(/DevTools listening on (ws:\/\/127\.0\.0\.1:\d+\/devtools\/browser\/[^\s]+)/)?.[1];
      if (url) resolve(url);
    });
    child.once('error', reject);
    child.once('exit', () => reject(new Error('Settings exited before renderer observation: ' + stderr.slice(-2000))));
  });
  browser = await chromium.connectOverCDP(debuggerUrl);
  const page = browser.contexts()[0].pages()[0];
  await page.locator('.settings-shell').waitFor();
  // Model a parent exiting: its read end closes before the child's native
  // close callback could send a user-dismiss event to that pipe.
  child.stdout.destroy();
  child.stdin.end();
  const deadline = new Promise<never>((_resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Retired Settings stayed alive')), 5000);
    timer.unref();
  });
  assert.equal(await Promise.race([exit, deadline]), 0);
});

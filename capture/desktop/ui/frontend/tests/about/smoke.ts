import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

test('Storage and About share one Electron window while pane capabilities remain scoped', async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'settings-panes-'));
  const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  const executable = createRequire(import.meta.url)('electron') as string;
  const application = spawn(executable, ['--no-sandbox', frontend, '--settings-storage', '--settings-parent', '--remote-debugging-port=0'], {
    env: { ...process.env, ELECTRON_RUN_AS_NODE: '', HOME: home, XDG_CONFIG_HOME: path.join(home, 'config'), XDG_CACHE_HOME: path.join(home, 'cache') },
    stdio: 'pipe',
  });
  let browser: Awaited<ReturnType<typeof chromium.connectOverCDP>> | undefined;
  const exited = new Promise<void>(resolve => application.once('exit', () => resolve()));
  try {
    const debuggerUrl = await new Promise<string>((resolve, reject) => {
      let stderr = '';
      application.stderr.on('data', chunk => {
        stderr += chunk.toString();
        const url = stderr.match(/DevTools listening on (ws:\/\/127\.0\.0\.1:\d+\/devtools\/browser\/[^\s]+)/)?.[1];
        if (url) resolve(url);
      });
      application.once('error', reject);
      application.once('exit', code => reject(new Error(`Settings exited before renderer observation (${code}): ${stderr.slice(-2000)}`)));
    });
    browser = await chromium.connectOverCDP(debuggerUrl);
    const page = browser.contexts()[0].pages()[0];
    assert.ok(page, 'the production Settings window opened');
    const pageTarget = await page.context().newCDPSession(page);
    const targetInfo = await pageTarget.send('Target.getTargetInfo');
    await expect(page.getByRole('heading', { name: 'Storage', level: 1 })).toBeVisible();
    if (process.env.APPARENT_SETTINGS_STORAGE_PROOF) await page.screenshot({ path: process.env.APPARENT_SETTINGS_STORAGE_PROOF });
    const pid = application.pid;
    await pageTarget.send('Emulation.setDeviceMetricsOverride', { width: 640, height: 530, deviceScaleFactor: 1, mobile: false });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true,
      'the Storage presentation fits at its minimum supported width');
    if (process.env.APPARENT_SETTINGS_NARROW_PROOF) await page.screenshot({ path: process.env.APPARENT_SETTINGS_NARROW_PROOF });
    assert.ok(await page.evaluate(() => window.apparent.readCaptureLocation()).then(value =>
      value && typeof value === 'object'), 'Storage can still read its bounded location state');
    let output = '';
    application.stdout.on('data', data => { output += data.toString(); });

    assert.equal(await page.evaluate(async () => {
      try { await window.apparentAbout.read(); return false; } catch { return true; }
    }), true, 'Storage cannot read About state');
    assert.equal(await page.evaluate(async () => {
      try { await window.apparentAbout.act({ action: 'check-for-updates' }); return false; } catch { return true; }
    }), true, 'Storage cannot invoke About actions');
    assert.equal(await page.evaluate(async () => {
      try { await window.apparent.navigateSettings('unknown'); return false; } catch { return true; }
    }), true, 'the navigation bridge rejects arbitrary destinations');

    await page.getByRole('button', { name: 'About', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect.poll(() => output.includes('"pane":"about"')).toBe(true);
    application.stdin.write('{"type":"settings-pane","pane":"about"}\n');
    const state = { type: 'about-state', version: '1.2.3', updates: true, linux: true,
      mainUpdates: true, updateChannel: 'main', updateCheck: { checking: false, result: 'available' } };
    application.stdin.write(`${JSON.stringify(state)}\n`);
    await expect(page.getByRole('heading', { name: 'About', level: 1 })).toBeVisible();
    await expect(page.getByText('Version 1.2.3')).toBeVisible();
    if (process.env.APPARENT_SETTINGS_ABOUT_PROOF) await page.screenshot({ path: process.env.APPARENT_SETTINGS_ABOUT_PROOF });
    assert.equal(await page.evaluate(async () => {
      try { await window.apparent.readCaptureLocation(); return false; } catch { return true; }
    }), true, 'About cannot invoke Storage actions');
    assert.equal(await page.evaluate(async () => {
      try { await window.apparentAbout.act({ action: 'set', key: 'microphoneEnabled', value: 'true' }); return false; }
      catch { return true; }
    }), true, 'About cannot invoke native or unrelated Settings actions');
    await page.getByRole('button', { name: 'Update later', exact: true }).click();
    await expect.poll(() => output.includes('"action":"update-later"')).toBe(true);

    await page.getByRole('button', { name: 'Storage', exact: true }).click();
    await expect.poll(() => output.includes('"pane":"storage"')).toBe(true);
    application.stdin.write('{"type":"settings-pane","pane":"storage"}\n');
    await expect(page.getByRole('heading', { name: 'Storage', level: 1 })).toBeVisible();
    assert.equal(await page.evaluate(async () => {
      try { await window.apparentAbout.read(); return false; } catch { return true; }
    }), true, 'About access is revoked after leaving About');
    assert.ok(await page.evaluate(() => window.apparent.readCaptureLocation()).then(value =>
      value && typeof value === 'object'), 'Storage actions work again after returning');
    assert.equal((await pageTarget.send('Target.getTargetInfo')).targetInfo.targetId, targetInfo.targetInfo.targetId,
      'pane navigation keeps the same renderer target and BrowserWindow');
    assert.equal(application.pid, pid, 'pane navigation keeps the Electron process alive');

    await page.getByRole('button', { name: 'About', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect.poll(() => output.match(/"pane":"about"/g)?.length ?? 0).toBe(2);
    application.stdin.write('{"type":"settings-pane","pane":"about"}\n');
    application.stdin.write(`${JSON.stringify(state)}\n`);
    await expect(page.getByRole('heading', { name: 'About', level: 1 })).toBeVisible();
    await page.getByRole('button', { name: 'Check for Updates…', exact: true }).click();
    await expect.poll(() => output.includes('"action":"check-for-updates"')).toBe(true);
    assert.equal(browser.contexts()[0].pages().length, 1, 'the original Settings window remains the only page');
    await page.getByRole('button', { name: 'General', exact: true }).click();
    await expect.poll(() => output.includes('"pane":"general"')).toBe(true,
      'native pane crossings remain parent-routed');
  } finally {
    await browser?.close().catch(() => {});
    if (application.exitCode === null) {
      application.kill('SIGTERM');
      await exited;
    }
    await fs.rm(home, { recursive: true, force: true });
  }
});

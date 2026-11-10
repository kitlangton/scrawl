import { test } from 'node:test';
import assert from 'node:assert/strict';
import { _electron as electron, expect } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

test('Storage and About share one Electron window while pane capabilities remain scoped', async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'settings-panes-'));
  const application = await electron.launch({
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config'), XDG_CACHE_HOME: path.join(home, 'cache') },
    args: [path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..'), '--settings-storage', '--settings-parent'],
  });
  try {
    const page = await application.firstWindow();
    await expect(page.getByRole('heading', { name: 'Storage', level: 1 })).toBeVisible();
    if (process.env.APPARENT_SETTINGS_STORAGE_PROOF) await page.screenshot({ path: process.env.APPARENT_SETTINGS_STORAGE_PROOF });
    const identity = await application.evaluate(({ BrowserWindow }) => ({
      ids: BrowserWindow.getAllWindows().map(window => window.id),
      hash: BrowserWindow.getAllWindows()[0].webContents.getURL().split('#')[1] ?? '',
    }));
    const pid = application.process().pid;
    await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(640, 530));
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true,
      'the Storage presentation fits at its minimum supported width');
    if (process.env.APPARENT_SETTINGS_NARROW_PROOF) await page.screenshot({ path: process.env.APPARENT_SETTINGS_NARROW_PROOF });
    assert.ok(await page.evaluate(() => window.apparent.readCaptureLocation()).then(value =>
      value && typeof value === 'object'), 'Storage can still read its bounded location state');
    let output = '';
    application.process().stdout?.on('data', data => { output += data.toString(); });

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
    application.process().stdin?.write('{"type":"settings-pane","pane":"about"}\n');
    const state = { type: 'about-state', version: '1.2.3', updates: true, linux: true,
      mainUpdates: true, updateChannel: 'main', updateCheck: { checking: false, result: 'available' } };
    application.process().stdin?.write(`${JSON.stringify(state)}\n`);
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
    application.process().stdin?.write('{"type":"settings-pane","pane":"storage"}\n');
    await expect(page.getByRole('heading', { name: 'Storage', level: 1 })).toBeVisible();
    assert.equal(await page.evaluate(async () => {
      try { await window.apparentAbout.read(); return false; } catch { return true; }
    }), true, 'About access is revoked after leaving About');
    assert.ok(await page.evaluate(() => window.apparent.readCaptureLocation()).then(value =>
      value && typeof value === 'object'), 'Storage actions work again after returning');
    await expect.poll(() => application.evaluate(({ BrowserWindow }) => ({
      ids: BrowserWindow.getAllWindows().map(window => window.id),
      hash: BrowserWindow.getAllWindows()[0].webContents.getURL().split('#')[1] ?? '',
    }))).toEqual({ ids: identity.ids, hash: identity.hash });
    assert.equal(application.process().pid, pid, 'pane navigation keeps the Electron process alive');

    await page.getByRole('button', { name: 'About', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect.poll(() => output.match(/"pane":"about"/g)?.length ?? 0).toBe(2);
    application.process().stdin?.write('{"type":"settings-pane","pane":"about"}\n');
    application.process().stdin?.write(`${JSON.stringify(state)}\n`);
    await expect(page.getByRole('heading', { name: 'About', level: 1 })).toBeVisible();
    await page.getByRole('button', { name: 'Check for Updates…', exact: true }).click();
    await expect.poll(() => output.includes('"action":"check-for-updates"')).toBe(true);
    assert.equal(await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length), 1,
      'the original BrowserWindow remains the only Settings window');
    await page.getByRole('button', { name: 'General', exact: true }).click();
    await expect.poll(() => output.includes('"pane":"general"')).toBe(true,
      'native pane crossings remain parent-routed');
  } finally { await application.close(); await fs.rm(home, { recursive: true, force: true }); }
});

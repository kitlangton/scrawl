import { test } from 'node:test';
import assert from 'node:assert/strict';
import { _electron as electron, expect } from '@playwright/test';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
test('installed About projects parent state and bounds actions', async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'about-'));
  const application = await electron.launch({ env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config'), XDG_CACHE_HOME: path.join(home, 'cache') }, args: [path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..'), '--settings-storage', '--settings-about'] });
  try {
    const page = await application.firstWindow();
    await expect(page.getByText('Loading update status…')).toBeVisible();
    let output = '';
    application.process().stdout?.on('data', data => { output += data.toString(); });
    const state = { type: 'about-state', version: '1.2.3', updates: true, linux: true,
      mainUpdates: true, updateChannel: 'main', updateCheck: { checking: false, result: 'available' } };
    await application.evaluate(({ BrowserWindow }, value) => BrowserWindow.getAllWindows()[0].webContents.send('apparent:about-state', value), state);
    await expect(page.getByText('Version 1.2.3')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Update now', exact: true })).toBeVisible();
    if (process.env.APPARENT_ABOUT_PROOF) await page.screenshot({ path: process.env.APPARENT_ABOUT_PROOF });
    await page.getByRole('button', { name: 'Update later', exact: true }).click();
    await expect.poll(() => output.includes('"action":"update-later"')).toBe(true);
    await page.getByRole('button', { name: 'Storage', exact: true }).click();
    await expect.poll(() => output.includes('"pane":"storage"')).toBe(true);
    assert.equal(await page.evaluate(async () => {
      try { await window.apparentAbout.act({ action: 'set', key: 'capturesDir', value: '/bad' }); return false; }
      catch { return true; }
    }), true);
    await application.evaluate(({ BrowserWindow }, value) => BrowserWindow.getAllWindows()[0].webContents.send('apparent:about-state', value),
      { ...state, updateCheck: { checking: true, result: null } });
    await expect(page.getByRole('button', { name: 'Checking…' })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Update now', exact: true })).toHaveCount(0);
  } finally { await application.close(); await fs.rm(home, { recursive: true, force: true }); }
});

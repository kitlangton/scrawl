// Run only in an owned disposable desktop, against the staged Settings package.
const { app, BrowserWindow, nativeTheme } = require('electron');
const fs = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
if (!process.env.SCRAWL_GUI_OUTPUT || !process.env.APPARENT_THEME_APP) throw Error('Disposable native theme test inputs required');
const run = (tool, args) => execFileSync(tool, args, { encoding: 'utf8' }).trim();
const mac = process.platform === 'darwin';
const read = () => mac ? run('osascript', ['-e', 'tell application "System Events" to tell appearance preferences to get dark mode']) : run('gsettings', ['get', 'org.gnome.desktop.interface', 'color-scheme']);
const set = value => mac ? run('osascript', ['-e', `tell application "System Events" to tell appearance preferences to set dark mode to ${value}`]) : run('gsettings', ['set', 'org.gnome.desktop.interface', 'color-scheme', value]);
const original = read();
require(path.join(process.env.APPARENT_THEME_APP, 'electron/settings.cjs'));
const until = async check => {
  const deadline = Date.now() + 12000;
  while (Date.now() < deadline) { if (await check()) return; await new Promise(r => setTimeout(r, 100)); }
  throw Error('System appearance did not reach the staged native window');
};
const deadline = setTimeout(() => { try { set(original); } finally { app.exit(1); } }, 60000);
app.whenReady().then(async () => {
  try {
    const window = BrowserWindow.getAllWindows()[0];
    await until(() => !window.webContents.isLoading());
    assert.equal(nativeTheme.themeSource, 'system');
    const results = [];
    for (const dark of [false, true, false]) {
      set(mac ? String(dark) : dark ? 'prefer-dark' : 'prefer-light');
      const background = dark ? 'rgb(32, 27, 36)' : 'rgb(245, 240, 232)';
      await until(async () => nativeTheme.shouldUseDarkColors === dark && await window.webContents.executeJavaScript('getComputedStyle(document.body).backgroundColor') === background);
      assert.equal(window.getBackgroundColor().slice(0, 7).toLowerCase(), dark ? '#201b24' : '#f5f0e8');
      const state = { dark, native: nativeTheme.shouldUseDarkColors, background, platform: process.platform, electron: process.versions.electron };
      results.push(state);
      await fs.writeFile(path.join(process.env.SCRAWL_GUI_OUTPUT, `native-${dark ? 'dark' : 'light'}.png`), (await window.webContents.capturePage()).toPNG());
    }
    await fs.writeFile(path.join(process.env.SCRAWL_GUI_OUTPUT, 'theme-native.json'), JSON.stringify(results, null, 2));
  } finally { set(original); clearTimeout(deadline); }
  app.quit();
}).catch(error => { console.error(error); app.exit(1); });

const { test } = require('node:test');
const assert = require('node:assert/strict');
const { _electron: electron, expect } = require('@playwright/test');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
test('Settings renderer and window follow Electron theme changes without reload', {timeout:30000}, async t => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'theme-'));
  const application = await electron.launch({args:[path.resolve(__dirname,'../..'),'--settings-storage'],env:{...process.env,HOME:home,XDG_CONFIG_HOME:path.join(home,'config')}});
  t.after(async()=>{await application.close();await fs.rm(home,{recursive:true,force:true});});
  const page = await application.firstWindow();
  // Playwright otherwise forces light media even when Electron follows the OS.
  await page.emulateMedia({colorScheme:null});
  await page.locator('.settings-shell').waitFor();
  assert.equal(await application.evaluate(({nativeTheme})=>nativeTheme.themeSource),'system');
  for(const mode of ['light','dark','light']) {
    await application.evaluate(({nativeTheme},mode)=>{nativeTheme.themeSource=mode;},mode);
    await expect(page.locator('body')).toHaveCSS('background-color',mode==='dark'?'rgb(32, 27, 36)':'rgb(245, 240, 232)');
    assert.equal(await application.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0].getBackgroundColor().slice(0,7).toLowerCase()),mode==='dark'?'#201b24':'#f5f0e8');
  }
  await application.evaluate(({nativeTheme})=>{nativeTheme.themeSource='system';});
});

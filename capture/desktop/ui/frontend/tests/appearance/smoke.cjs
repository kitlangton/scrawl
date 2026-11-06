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
  await application.evaluate(({BrowserWindow, ipcMain}) => {
    global.settingsStartupObservation = { paintObserved: false, mountedReceived: false, mountedAccepted: false, frameReadyReceived: false, frameReadyAccepted: false };
    BrowserWindow.getAllWindows()[0].once('ready-to-show', () => { global.settingsStartupObservation.paintObserved = true; });
    // Observe the existing handler without replacing its authorization or result.
    const handlers = ipcMain._invokeHandlers;
    const mounted = handlers?.get('apparent:settings-mounted');
    if (typeof mounted === 'function') handlers.set('apparent:settings-mounted', (...args) => {
      global.settingsStartupObservation.mountedReceived = true;
      const result = mounted(...args);
      global.settingsStartupObservation.mountedAccepted = true;
      return result;
    });
    const frameReady = handlers?.get('apparent:settings-frame-ready');
    if (typeof frameReady === 'function') handlers.set('apparent:settings-frame-ready', (...args) => {
      global.settingsStartupObservation.frameReadyReceived = true;
      const result = frameReady(...args);
      global.settingsStartupObservation.frameReadyAccepted = true;
      return result;
    });
  });
  // Observe startup before DevTools media emulation can invalidate its first paint.
  await page.locator('.settings-shell').waitFor();
  try {
    await expect.poll(() => application.evaluate(({BrowserWindow}) => BrowserWindow.getAllWindows()[0].isVisible())).toBe(true);
  } catch (error) {
    t.diagnostic(JSON.stringify(await application.evaluate(({BrowserWindow, app}) => {
      const w = BrowserWindow.getAllWindows()[0];
      return { visible: w.isVisible(), destroyed: w.isDestroyed(), loading: w.webContents.isLoading(), crashed: w.webContents.isCrashed(), readiness: global.settingsStartupObservation, gpu: app.getGPUFeatureStatus() };
    })));
    t.diagnostic(JSON.stringify(await page.evaluate(() => ({ readyState: document.readyState, visibility: document.visibilityState,
      mountedBridge: typeof window.apparentSettingsMounted, shellPresent: !!document.querySelector('.settings-shell') }))));
    throw error;
  }
  const startupObservation = await application.evaluate(() => global.settingsStartupObservation);
  assert.equal(startupObservation.mountedReceived, true, 'The real Settings mount handler ran');
  assert.equal(startupObservation.mountedAccepted, true, 'The Settings mount handler accepted the shipped preload');
  assert.equal(startupObservation.frameReadyReceived, true, 'The shipped preload sent a post-commit frame acknowledgement');
  assert.equal(startupObservation.frameReadyAccepted, true, 'The Settings host accepted the post-commit frame');
  // Playwright otherwise forces light media even when Electron follows the OS.
  await page.emulateMedia({colorScheme:null});
  const contentBounds = await page.locator('.apparent-window-content').boundingBox();
  const shellBounds = await page.locator('.settings-shell').boundingBox();
  assert.equal(shellBounds.height, contentBounds.height, 'Settings must fit above the build label without an outer scroll area');
  await expect(page.getByText('CAPTURE SETTINGS', {exact:true})).toBeVisible();
  assert.equal(await application.evaluate(({nativeTheme})=>nativeTheme.themeSource),'system');
  for(const mode of ['light','dark','light']) {
    await application.evaluate(({nativeTheme},mode)=>{nativeTheme.themeSource=mode;},mode);
    await expect(page.locator('body')).toHaveCSS('background-color',mode==='dark'?'rgb(32, 27, 36)':'rgb(245, 240, 232)');
    await expect(page.locator('.settings-sidebar')).toHaveCSS('background-color','rgb(2, 1, 2)');
    await expect(page.locator('.settings-sidebar')).toHaveCSS('color','rgb(245, 240, 232)');
    assert.equal(await application.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows()[0].getBackgroundColor().slice(0,7).toLowerCase()),mode==='dark'?'#201b24':'#f5f0e8');
  }
  await application.evaluate(({nativeTheme})=>{nativeTheme.themeSource='system';});
});

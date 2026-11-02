// Run with the Electron binary in an owned disposable desktop, not on a real screen.
const { app, BrowserWindow } = require('electron');
const fs = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const output = process.env.SCRAWL_GUI_OUTPUT;
if (!output) throw new Error('Run only inside the isolated GUI runner');
require('../electron/main.cjs');

const timeout = setTimeout(() => { console.error('Native smoke deadline'); app.exit(1); }, 30000);
app.whenReady().then(async () => {
  const window = BrowserWindow.getAllWindows()[0];
  const until = Date.now() + 15000;
  let text = '';
  while (Date.now() < until) {
    try { text = await window.webContents.executeJavaScript('document.body.innerText'); } catch {}
    if (text.includes('Not connected') || text.includes('Connected ·')) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(text.includes('Live Rust state'));
  const bridge = await window.webContents.executeJavaScript('window.apparent.readRuntimeState()');
  if (process.env.NATIVE_EXPECT_CONNECTED === '1') assert.equal(bridge.status, 'connected');
  else assert.equal(bridge.status, 'disconnected');
  for (const label of ['Recording', 'Processing', 'Uploading', 'Preparing', 'Permission', 'Error']) {
    const point = await window.webContents.executeJavaScript(`(() => {
      const rect = Array.from(document.querySelectorAll('[role="tab"]')).find(e => e.textContent === ${JSON.stringify(label)}).getBoundingClientRect();
      return { x: Math.round(rect.x + rect.width / 2), y: Math.round(rect.y + rect.height / 2) };
    })()`);
    window.webContents.sendInputEvent({ type: 'mouseDown', button: 'left', clickCount: 1, ...point });
    window.webContents.sendInputEvent({ type: 'mouseUp', button: 'left', clickCount: 1, ...point });
    await new Promise((resolve) => setTimeout(resolve, 100));
    assert.equal(await window.webContents.executeJavaScript('document.querySelector(\'[role="tab"][aria-selected="true"]\').textContent'), label);
  }
  const image = await window.webContents.capturePage();
  await fs.writeFile(path.join(output, 'electron-native.png'), image.toPNG());
  await fs.writeFile(path.join(output, 'electron-native.json'), JSON.stringify({
    bridge, pid: process.pid, title: window.getTitle(), electron: process.versions.electron,
    platform: process.platform, display: process.env.DISPLAY, wayland: process.env.WAYLAND_DISPLAY,
    sandbox: window.webContents.getLastWebPreferences().sandbox,
    liveRust: bridge.status === 'connected',
  }, null, 2));
  clearTimeout(timeout);
  app.quit();
}).catch((error) => { console.error(error); app.exit(1); });

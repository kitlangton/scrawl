const { test } = require('node:test');
const assert = require('node:assert/strict');
const { _electron: electron, expect } = require('@playwright/test');
const fs = require('node:fs/promises');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');

// Real shipped main/preload/renderer; only the socket and OS dialog are synthetic.
// Rust policy is exercised separately through the actual Rust endpoint.
test('Storage IPC, cancel, reset/readback, stale reads and reload invalidate picker', { timeout: 45000 }, async (t) => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  await fs.chmod(directory, 0o700);
  const requests = [];
  let saved = '/saved';
  let revision = 'a'.repeat(16);
  let holdRead = false;
  let releaseRead;
  const snapshot = () => ({ savedPath: saved, effectivePath: saved ?? '/builtin', displayPath: saved ?? '/builtin',
    custom: saved !== null, source: saved ? 'configured' : 'builtin', revision, secret: 'hidden' });
  const server = net.createServer((socket) => socket.once('data', (data) => {
    if (data.toString() === 'state\n') return socket.end('{"state":{"phase":"idle"}}\n');
    const request = JSON.parse(data.toString());
    requests.push(request);
    if (request.storage === 'read' && holdRead) {
      holdRead = false;
      const old = snapshot();
      releaseRead = () => socket.end(JSON.stringify({ storage: old }) + '\n');
      return;
    }
    if (request.storage === 'set') { saved = request.path; revision = 'b'.repeat(16); }
    if (request.storage === 'reset') { saved = null; revision = 'c'.repeat(16); }
    socket.end(JSON.stringify({ storage: snapshot(), secret: 'hidden' }) + '\n');
  }));
  const address = path.join(directory, 'capture.sock');
  await new Promise((resolve) => server.listen(address, resolve));
  await fs.chmod(address, 0o600);
  const application = await electron.launch({ args: [path.join(__dirname, '..')],
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config') } });
  t.after(async () => {
    releaseRead?.();
    await application.close();
    await new Promise((resolve) => server.close(resolve));
    await fs.rm(home, { recursive: true, force: true });
  });
  const page = await application.firstWindow();
  await page.getByText('/saved', { exact: true }).waitFor();
  assert.equal((await page.evaluate(() => window.apparent.readCaptureLocation())).storage.revision, undefined);
  assert.equal((await page.evaluate(() => window.apparent.readCaptureLocation())).storage.secret, undefined);
  await application.evaluate(({ dialog, BrowserWindow }) => {
    global.pickers = [];
    dialog.showOpenDialog = (parent, options) => {
      if (parent !== BrowserWindow.getAllWindows()[0]) throw new Error('unparented picker');
      global.pickers.push(options);
      return new Promise((resolve) => { global.resolvePicker = resolve; });
    };
  });
  const mutations = () => requests.filter((r) => r.storage !== 'read').length;
  const waitPicker = async (count) => expect.poll(() => application.evaluate(() => global.pickers.length)).toBe(count);
  await page.getByRole('button', { name: 'Change…', exact: true }).click();
  await waitPicker(1);
  assert.deepEqual(await page.evaluate(() => window.apparent.resetCaptureLocation()), { status: 'error', code: 'app_busy' });
  await application.evaluate(() => global.resolvePicker({ canceled: true, filePaths: [] }));
  await expect(page.getByRole('button', { name: 'Change…', exact: true })).toBeEnabled();
  assert.equal(mutations(), 0);
  const options = await application.evaluate(() => global.pickers[0]);
  assert.deepEqual(options.properties, ['openDirectory', 'createDirectory']);
  assert.equal(options.defaultPath, '/');

  // A read started before a save may finish afterward, but must not publish old state.
  holdRead = true;
  await page.evaluate(() => { window.oldRead = window.apparent.readCaptureLocation(); });
  await expect.poll(() => typeof releaseRead).toBe('function');
  await page.getByRole('button', { name: 'Change…', exact: true }).click();
  await waitPicker(2);
  await application.evaluate(() => global.resolvePicker({ canceled: false, filePaths: ['/new é captures'] }));
  await page.getByText('/new é captures', { exact: true }).waitFor();
  releaseRead(); releaseRead = undefined;
  assert.deepEqual(await page.evaluate(() => window.oldRead), { status: 'cancelled' });
  assert.equal(mutations(), 1);
  assert.equal(requests.find((r) => r.storage === 'set').expected, 'a'.repeat(16));
  await page.getByRole('button', { name: 'Reset', exact: true }).click();
  await page.getByText('/builtin', { exact: true }).waitFor();
  await expect(page.getByRole('button', { name: 'Reset', exact: true })).toBeDisabled();
  assert.equal(mutations(), 2);

  await page.getByRole('button', { name: 'Change…', exact: true }).click();
  await waitPicker(3);
  await page.reload();
  await page.getByText('/builtin', { exact: true }).waitFor();
  await application.evaluate(() => global.resolvePicker({ canceled: false, filePaths: ['/must-not-save'] }));
  await expect(page.getByRole('button', { name: 'Change…', exact: true })).toBeEnabled();
  assert.equal(mutations(), 2);
  assert.equal(await page.getByText('/must-not-save', { exact: true }).count(), 0);
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(640, 760));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
  await page.getByRole('button', { name: 'Change…', exact: true }).focus();
  await expect(page.getByRole('button', { name: 'Change…', exact: true })).toBeFocused();

  assert.equal(await application.evaluate(async ({ BrowserWindow }, root) => {
    const other = new BrowserWindow({ show: false, webPreferences: {
      preload: root + '/electron/preload.cjs', sandbox: true, contextIsolation: true, nodeIntegration: false,
    } });
    try {
      await other.loadFile(root + '/dist/index.html');
      return await other.webContents.executeJavaScript(`Promise.all(['readCaptureLocation','changeCaptureLocation','resetCaptureLocation'].map(k => window.apparent[k]().then(() => false, () => true))).then(r => r.every(Boolean))`);
    } finally { other.destroy(); }
  }, path.join(__dirname, '..')), true);
  // Invoke shipped handlers with an owned sender but extra args / non-main frame.
  assert.equal(await application.evaluate(async ({ ipcMain, BrowserWindow }) => {
    const sender = BrowserWindow.getAllWindows()[0].webContents;
    for (const channel of ['read', 'change', 'reset']) {
      const handler = ipcMain._invokeHandlers.get(`apparent:${channel}-capture-location`);
      for (const [frame, args] of [[sender.mainFrame, ['/arbitrary']], [{ url: sender.getURL() }, []]]) {
        try { await handler({ sender, senderFrame: frame }, ...args); return false; } catch {}
      }
    }
    return true;
  }), true);
  const screenshot = path.join(os.tmpdir(), `apparent-storage-${process.pid}.png`);
  await page.screenshot({ path: screenshot, fullPage: true });
  console.log(`Synthetic Storage screenshot: ${screenshot}`);

  // Keep the real owned sender/frame, and await the shipped handler directly so
  // navigation/closing its renderer cannot hide the result from the test.
  const beginLifecyclePicker = () => application.evaluate(({ ipcMain, BrowserWindow }) => {
    const sender = BrowserWindow.getAllWindows()[0].webContents;
    global.lifecycleMutation = ipcMain._invokeHandlers.get('apparent:change-capture-location')({
      sender, senderFrame: sender.mainFrame,
    });
  });
  await beginLifecyclePicker();
  await waitPicker(4);
  assert.deepEqual(await application.evaluate(async ({ BrowserWindow }) => {
    const sender = BrowserWindow.getAllWindows()[0].webContents;
    const original = sender.getURL();
    const navigation = new Promise(resolve => sender.once('will-navigate', resolve));
    await sender.executeJavaScript(`(() => { const a = document.createElement('a'); a.href = 'https://example.com'; document.body.append(a); a.click(); a.remove(); })()`);
    await navigation;
    if (sender.getURL() !== original) throw new Error('Navigation escaped');
    global.resolvePicker({ canceled: false, filePaths: ['/must-not-save-after-navigation'] });
    return await global.lifecycleMutation;
  }), { status: 'cancelled' });
  assert.equal(mutations(), 2);
  await beginLifecyclePicker();
  await waitPicker(5);
  assert.deepEqual(await application.evaluate(async ({ BrowserWindow }) => {
    const owned = BrowserWindow.getAllWindows()[0];
    // Keep the app alive long enough to resolve and inspect its pending dialog.
    new BrowserWindow({ show: false });
    owned.destroy();
    global.resolvePicker({ canceled: false, filePaths: ['/must-not-save-after-close'] });
    return await global.lifecycleMutation;
  }), { status: 'cancelled' });
  assert.equal(mutations(), 2);
});

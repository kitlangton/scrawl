const { test } = require('node:test');
const assert = require('node:assert/strict');
const { _electron: electron, expect } = require('@playwright/test');
const fs = require('node:fs/promises');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
// Transport/UI security fixture only. Actual consent policy is tested in Rust.
test('Connect shipped Electron IPC, consent presentation, browser action and lifecycle ownership', { timeout: 45000 }, async t => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ec-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  await fs.chmod(directory, 0o700);
  const clients = new Set(), requests = [];
  let closes = 0, stage = 'initial', hold, holdAction;
  const button = (command, title) => ({ command, title, enabled: true });
  const view = () => ({ title: 'Apparent Connect', status: stage === 'approval' ? 'Waiting for approval' : 'Not connected',
    detail: stage === 'consent' ? 'Explicit upload consent required.' : 'Synthetic transport fixture.',
    server: 'https://api.example.com', note: 'No old captures or public links.',
    userCode: stage === 'approval' ? 'TEST-CODE' : null,
    verificationUri: stage === 'approval' ? 'https://example.com/device?code=TEST-CODE' : null,
    buttons: stage === 'initial' ? [button('connect', 'Sign in')] : stage === 'consent'
      ? [button('allow', 'Allow uploads'), button('cancel', 'Cancel')]
      : [button('open-browser', 'Open browser'), button('cancel', 'Cancel')] });
  const server = net.createServer(socket => {
    clients.add(socket); socket.on('error', () => {});
    let connect = false, bytes = '';
    socket.on('close', () => { clients.delete(socket); if (connect) closes += 1; });
    socket.on('data', chunk => {
      bytes += chunk;
      while (bytes.includes('\n')) {
        const end = bytes.indexOf('\n'), line = bytes.slice(0, end); bytes = bytes.slice(end + 1);
        if (line === 'state') { socket.end('{"state":{"phase":"idle"}}\n'); continue; }
        const request = JSON.parse(line);
        if (request.storage) { socket.end('{"error":"feature_disabled"}\n'); continue; }
        connect = true; requests.push(request);
        if (request.connect === 'settings') stage = 'initial';
        if (request.command === 'connect') stage = 'consent';
        if (request.command === 'allow') stage = 'approval';
        if (request.command === 'cancel') stage = 'initial';
        const response = { connect: { ...view(), privateCredential: 'synthetic-secret' },
          ...(request.command === 'open-browser' ? { openUri: view().verificationUri } : {}) };
        if (request.command && request.command === holdAction) { hold = () => socket.write(JSON.stringify(response) + '\n'); continue; }
        socket.write(JSON.stringify(response) + '\n');
      }
    });
  });
  const address = path.join(directory, 'capture.sock');
  await new Promise(resolve => server.listen(address, resolve)); await fs.chmod(address, 0o600);
  const application = await electron.launch({ args: [path.join(__dirname, '..')],
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config') } });
  t.after(async () => {
    await application.close(); for (const client of clients) client.destroy();
    await new Promise(resolve => server.close(resolve)); await fs.rm(home, { recursive: true, force: true });
  });
  await application.evaluate(({ shell }) => { global.openedUris = []; shell.openExternal = async uri => { global.openedUris.push(uri); }; });
  const page = await application.firstWindow();
  const card = page.getByRole('region', { name: 'Apparent Connect' });
  await card.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(card.getByText('Explicit upload consent required.')).toBeVisible();
  assert.equal(requests.filter(r => r.command === 'allow').length, 0);
  assert.deepEqual(await application.evaluate(() => global.openedUris), []);
  await card.getByRole('button', { name: 'Allow uploads', exact: true }).click();
  await expect(card.getByText('TEST-CODE', { exact: true })).toBeVisible();
  assert.deepEqual(await application.evaluate(() => global.openedUris), []);
  assert.equal((await page.evaluate(() => window.apparent.readConnectSettings())).view.privateCredential, undefined);
  await card.getByRole('button', { name: 'Open browser', exact: true }).click();
  await expect.poll(() => application.evaluate(() => global.openedUris.length)).toBe(1);
  await card.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(card.getByRole('button', { name: 'Sign in', exact: true })).toBeEnabled();
  assert.equal(requests.filter(r => r.command === 'cancel').length, 1);
  // All shipped Connect handlers reject extra args, wrong frames and other windows.
  assert.equal(await application.evaluate(async ({ ipcMain, BrowserWindow }, root) => {
    const owner = BrowserWindow.getAllWindows()[0].webContents;
    for (const [name, handler] of ipcMain._invokeHandlers) {
      if (!name.startsWith('apparent:connect-') && name !== 'apparent:read-connect-settings') continue;
      for (const [sender, frame, args] of [[owner, owner.mainFrame, ['https://arbitrary.example/']], [owner, { url: owner.getURL() }, []]]) {
        try { await handler({ sender, senderFrame: frame }, ...args); return false; } catch {}
      }
    }
    const other = new BrowserWindow({ show: false, webPreferences: { preload: root + '/electron/preload.cjs', sandbox: true, contextIsolation: true, nodeIntegration: false } });
    try {
      await other.loadFile(root + '/dist/index.html');
      return await other.webContents.executeJavaScript(`Promise.all(['readConnectSettings','connectSettings','allowConnectUploads','cancelConnectSettings','checkConnectConnection','disableConnectUploads','disconnectConnectSettings','openConnectBrowser'].map(k => window.apparent[k]().then(() => false, () => true))).then(r => r.every(Boolean))`);
    } finally { other.destroy(); }
  }, path.join(__dirname, '..')), true);
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(640, 760));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
  // A pending browser approval response cannot launch after renderer replacement.
  await card.getByRole('button', { name: 'Sign in', exact: true }).click();
  await card.getByRole('button', { name: 'Allow uploads', exact: true }).click();
  holdAction = 'open-browser';
  await application.evaluate(({ ipcMain, BrowserWindow }) => {
    const sender = BrowserWindow.getAllWindows()[0].webContents;
    global.oldConnect = ipcMain._invokeHandlers.get('apparent:connect-open-browser')({ sender, senderFrame: sender.mainFrame });
  });
  await expect.poll(() => typeof hold).toBe('function');
  await page.reload();
  assert.deepEqual(await application.evaluate(() => global.oldConnect), { status: 'cancelled' });
  await expect.poll(() => closes).toBe(1);
  holdAction = undefined; hold();
  await expect(card.getByRole('button', { name: 'Sign in', exact: true })).toBeVisible();
  assert.deepEqual(await application.evaluate(() => global.openedUris), ['https://example.com/device?code=TEST-CODE']);
  assert.equal(await page.getByText('synthetic-secret').count(), 0);
  const shot = path.join(os.tmpdir(), `apparent-connect-${process.pid}.png`);
  await page.screenshot({ path: shot, fullPage: true });
  console.log(`Synthetic Connect screenshot: ${shot}`);
});

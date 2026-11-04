const { test } = require('node:test');
const assert = require('node:assert/strict');
const { _electron: electron, expect } = require('@playwright/test');
const fs = require('node:fs/promises');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');

// Exercise the real Electron main/preload/renderer against an explicitly synthetic
// socket. This is transport/security proof, not a live Rust or native-platform pass.
test('Electron preview, read-only bridge and isolation', { timeout: 45000 }, async (t) => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  const requests = [];
  let reply = '{"state":{"phase":"idle","capture_id":null}}\n';
  let reportingEnabled = true;
  let reportingReads = 0;
  let reportingAlwaysBusy = false;
  let reportingWrites = 0;
  let rejectReportingWrite = true;
  let publicFailure=false;
  let server;
  const application = await electron.launch({
    args: [path.join(__dirname, '..')],
    ...(process.env.PUBLIC_SHARE_VIDEO ? {recordVideo:{dir:process.env.PUBLIC_SHARE_VIDEO,size:{width:920,height:760}}}:{}),
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config') },
  });
  t.after(async () => {
    await application.close();
    if(process.env.PUBLIC_SHARE_VIDEO && page.video()) await page.video().saveAs(path.join(process.env.PUBLIC_SHARE_VIDEO,'sender-electron.webm'));
    if (server) await new Promise((resolve) => server.close(resolve));
    await fs.rm(home, { recursive: true, force: true });
  });
  const page = await application.firstWindow();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.getByText('Not connected', { exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => ({
    keys: Object.keys(window.apparent), require: typeof window.require, process: typeof window.process,
  })), { keys: ['readDrawing', 'undoDrawing', 'readMicrophoneEnabled', 'setMicrophoneEnabled', 'readRuntimeState', 'readCaptureReference', 'copyCaptureReference', 'startPublicShare', 'readPublicShare', 'cancelPublicShare', 'readCaptureLocation', 'changeCaptureLocation', 'resetCaptureLocation',
    'readErrorReporting', 'setErrorReporting', 'readConnectSettings', 'connectSettings', 'allowConnectUploads', 'cancelConnectSettings', 'checkConnectConnection',
    'disableConnectUploads', 'disconnectConnectSettings', 'requestDisableConnectUploads', 'requestDisconnectConnectSettings', 'openConnectSharedLinks',
    'openConnectBrowser', 'readRetention', 'saveRetention', 'resetRetention', 'readCaptureUsage', 'navigateSettings', 'showCaptureFolder'], require: 'undefined', process: 'undefined' });
  assert.equal(await application.evaluate(({ BrowserWindow }) => {
    const options = BrowserWindow.getAllWindows()[0].webContents.getLastWebPreferences();
    return options.contextIsolation && options.sandbox && !options.nodeIntegration && options.webSecurity;
  }), true);
  assert.equal(await page.evaluate(async () => {
    try { await fetch('https://example.com'); return false; } catch { return true; }
  }), true);

  for (const label of ['Recording', 'Processing', 'Uploading', 'Preparing', 'Permission', 'Error', 'Idle']) {
    await page.getByRole('tab', { name: label, exact: true }).click();
    await page.getByRole('tabpanel').getByText('SYNTHETIC EXAMPLE', { exact: true }).waitFor();
    await expect(page.getByRole('tab', { name: label, exact: true })).toHaveAttribute('aria-selected', 'true');
  }
  await page.getByRole('tab', { name: 'Idle', exact: true }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Recording', exact: true })).toHaveAttribute('aria-selected', 'true');
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(640, 760));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);

  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  await fs.chmod(directory, 0o700);
  server = net.createServer((socket) => socket.once('data', (data) => {
    requests.push(data.toString());
    const request = data.toString().startsWith('{') ? JSON.parse(data.toString()) : null;
    if (request?.drawing) {
      if (request.drawing === 'undo') assert.equal(request.expected, '0123456789abcdef');
      socket.end(JSON.stringify({ drawing: { revision: '0123456789abcdef', strokes: request.drawing === 'undo' ? 0 : 1 } }) + '\n');
    } else if (request?.reference) {
      socket.end('{"reference":{"id":"synthetic.apparent","copied":false}}\n');
    } else if(request?.publicShare) {
      if(request.publicShare==='deliver' && publicFailure) socket.end('{"error":"clipboard_failed"}\n');
      else socket.end(JSON.stringify({publicShare:request.publicShare==='start'?{operation:'a'.repeat(32),state:'preparing'}:{state:request.publicShare==='read'?'prepared':request.publicShare==='deliver'?'copied':'cancelled'}})+'\n');
    } else if (request?.errorReporting) {
      if (request.errorReporting === 'read' && (++reportingReads <= 3 || reportingAlwaysBusy)) {
        socket.end('{"error":"app_busy"}\n');
        return;
      }
      if (request.errorReporting === 'set') {
        reportingWrites += 1;
        if (rejectReportingWrite) return socket.end('{"error":"app_busy"}\n');
        reportingEnabled = request.enabled;
      }
      socket.end(JSON.stringify({ errorReporting: { enabled: reportingEnabled } }) + '\n');
    } else socket.end(reply);
  }));
  const address = path.join(directory, 'capture.sock');
  await new Promise((resolve) => server.listen(address, resolve));
  await fs.chmod(address, 0o600);
  // Let the production one-second cooldown expire, rather than bypassing it.
  await new Promise((resolve) => setTimeout(resolve, 1100));
  await page.getByRole('button', { name: 'Refresh state' }).click();
  await page.getByText('Connected · idle', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Refresh drawing', exact: true }).click();
  await page.getByText('1 marks · screen recording active', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Undo last mark', exact: true }).click();
  await page.getByText('0 marks · screen recording active', { exact: true }).waitFor();
  await expect(page.getByRole('button', { name: 'Undo last mark', exact: true })).toBeDisabled();
  assert.equal(requests.filter(r => r.includes('"drawing":"undo"')).length, 1);

  const reporting = page.getByRole('region', { name: 'Send technical error reports', exact: true });
  await reporting.getByRole('button', { name: 'Refresh reporting setting' }).click();
  const toggle = reporting.getByRole('checkbox', { name: 'Send technical error reports' });
  await expect(toggle).toBeChecked();
  assert.equal(reportingReads, 4, 'repeated busy reads recover without another human action');
  await toggle.click();
  await expect(reporting.getByText('Rust is handling another settings request. Refresh shortly.')).toBeVisible();
  assert.equal(reportingWrites, 1, 'a rejected save must never be repeated');
  rejectReportingWrite = false;
  await reporting.getByRole('button', { name: 'Refresh reporting setting' }).click();
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  assert.equal(reportingEnabled, false);
  const beforeBusy = reportingReads;
  reportingAlwaysBusy = true;
  await reporting.getByRole('button', { name: 'Refresh reporting setting' }).click();
  await expect(reporting.getByText('Rust is handling another settings request. Refresh shortly.')).toBeVisible();
  await expect(reporting.getByRole('button', { name: 'Refresh reporting setting' })).toBeEnabled();
  assert.equal(reportingReads - beforeBusy, 6, 'persistent contention stops after the bounded read attempts');
  reportingAlwaysBusy = false;
  assert.equal(await page.evaluate(async () => {
    try { await window.apparent.setErrorReporting('false'); return false; } catch { return true; }
  }), true);
  reply = '{"error":"app_busy"}\n';
  await new Promise((resolve) => setTimeout(resolve, 1100));
  await page.getByRole('button', { name: 'Refresh state' }).click();
  await page.getByText('Unable to read state', { exact: true }).waitFor();
  reply = '{"state":{"phase":"processing","capture_id":"private"}}\n';
  await new Promise((resolve) => setTimeout(resolve, 1100));
  await page.reload();
  await page.getByText('Connected · processing', { exact: true }).waitFor();
  assert.equal(await page.getByText('private', { exact: true }).count(), 0);
  assert.ok(requests.length >= 3);
  assert.ok(requests.every((request) => request === 'state\n' || ['read', 'undo'].includes(JSON.parse(request).drawing) || JSON.parse(request).microphone === 'read' || ['read', 'locationfacts', 'probelocation', 'retention', 'usage'].includes(JSON.parse(request).storage) || JSON.parse(request).reference === 'read' || JSON.parse(request).connect === 'settings' ||
    (JSON.parse(request).connect === 'backend' && JSON.parse(request).version === 1 &&
      Object.keys(JSON.parse(request)).every((key) => ['connect', 'version', 'deadline'].includes(key))) ||
    ['read', 'set'].includes(JSON.parse(request).errorReporting)));
  assert.deepEqual(errors, []);
  const screenshot = path.join(os.tmpdir(), `apparent-foundation-${process.pid}.png`);
  await page.screenshot({ path: screenshot, fullPage: true });
  console.log(`Synthetic Electron screenshot: ${screenshot}`);

  await page.getByRole('button',{name:'Refresh capture reference',exact:true}).click();
  await page.getByText('synthetic.apparent',{exact:true}).waitFor();
  const beforeShare=requests.filter(r=>r.includes('"publicShare"')).length;
  await page.getByRole('button',{name:'Copy public link',exact:true}).click();
  await expect(page.getByText('Anyone with this link can view this capture. Shared captures are not end-to-end encrypted.',{exact:true})).toBeVisible();
  assert.equal(requests.filter(r=>r.includes('"publicShare"')).length,beforeShare);
  if(process.env.PUBLIC_SHARE_VIDEO) await page.waitForTimeout(1500);
  await page.getByRole('button',{name:'Create and copy public link',exact:true}).click();
  await page.getByText('Link copied · upload may still be preparing. Open the link to check readiness.',{exact:true}).waitFor();
  if(process.env.PUBLIC_SHARE_VIDEO) {await page.screenshot({path:path.join(process.env.PUBLIC_SHARE_VIDEO,'sender-copied.png')});await page.waitForTimeout(1500);}
  assert.equal(requests.filter(r=>r.includes('"publicShare":"deliver"')).length,1);
  publicFailure=true;
  await page.getByRole('button',{name:'Copy public link',exact:true}).click();
  await page.getByRole('button',{name:'Create and copy public link',exact:true}).click();
  await page.getByText('Public link was not confirmed copied. Check your clipboard and refresh before trying again. Local Paste remains available.',{exact:true}).waitFor();

  // Do this after interaction checks: Playwright tracks even a cancelled Electron
  // navigation as pending; the actual webContents URL remains authoritative.
  const originalUrl = page.url();
  await page.evaluate(() => {
    const link = document.createElement('a');
    link.href = 'https://example.com'; document.body.append(link); link.click(); link.remove();
    window.open('https://example.com');
  });
  assert.equal(await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].webContents.getURL()), originalUrl);
  assert.equal(application.windows().length, 1);

  // A second renderer with the same preload still cannot invoke the bridge.
  assert.equal(await application.evaluate(async ({ BrowserWindow }, root) => {
    const other = new BrowserWindow({ show: false, webPreferences: {
      preload: root + '/electron/preload.cjs', sandbox: true, contextIsolation: true, nodeIntegration: false,
    } });
    try {
      await other.loadFile(root + '/dist/index.html');
      return await other.webContents.executeJavaScript(`Promise.all(['readRuntimeState', 'readCaptureReference', 'copyCaptureReference', 'startPublicShare', 'readPublicShare', 'cancelPublicShare'].map(k => window.apparent[k]().then(() => false, () => true))).then(r => r.every(Boolean))`);
    } finally { other.destroy(); }
  }, path.join(__dirname, '..')), true);
});

// Shared identity rendering and click-through checks use the shipped renderer.
require('./build/identity.smoke.ts');

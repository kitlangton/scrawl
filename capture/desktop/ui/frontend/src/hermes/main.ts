import { app, BrowserWindow, ipcMain, session } from 'electron';
import { execFile, spawn } from 'node:child_process';
import { mkdir, open } from 'node:fs/promises';
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import { Recording, destination, type Receipt, type View } from './protocol';
import { mailbox } from './mailbox';
import { keyDirectory, publishKey } from './key';
const option = (key: string) => process.argv[process.argv.indexOf(key) + 1];
const request = destination(option('--hermes-request'));
const executable = option('--apparent-executable');
if (!path.isAbsolute(executable)) throw new Error('Invalid launch request');
app.setName('Apparent for Hermes');
app.setPath('userData', path.join(app.getPath('appData'), 'apparent-hermes'));
let view: View = { phase: 'confirm' };
let window: BrowserWindow;
let running = false;
const showWindow = () => {
  if (window.isDestroyed()) return;
  if (process.platform === 'darwin') app.show();
  window.show();
};
const owned = new Recording(request.token, (args) => new Promise<Receipt>((resolve, reject) => {
  execFile(executable, args, { timeout: 35000, maxBuffer: 65536, encoding: 'utf8' }, (_error, stdout) => {
    try { resolve(JSON.parse(stdout)); } catch { reject(new Error('Apparent is unavailable. Open the installed app and finish onboarding.')); }
  });
}));
const sender = (event: Electron.IpcMainInvokeEvent) => {
  if (event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame) throw new Error('Invalid sender');
};
const ready = () => new Promise<void>((resolve, reject) => execFile(executable, ['capture', 'state'], { timeout: 4500, maxBuffer: 65536 }, error => error ? reject(error) : resolve()));
app.whenReady().then(async () => {
  // Duplicate launches bind the same port and fail before creating a window or recording.
  const key = randomBytes(16).toString('hex');
  const server = await mailbox(request.token, request.port, key, () => view, () => window.close());
  app.once('before-quit', () => server.close());
  const removeKey = publishKey(keyDirectory(), request.token, key);
  app.once('before-quit', removeKey);
  const deadline = setTimeout(() => { server.close(); if (!window.isDestroyed()) window.close(); }, 60 * 60 * 1000);
  app.once('before-quit', () => clearTimeout(deadline));
  session.defaultSession.setPermissionRequestHandler((_w, _p, reply) => reply(false));
  session.defaultSession.setPermissionCheckHandler(() => false);
  window = new BrowserWindow({ width: 500, height: 340, resizable: false, show: false, autoHideMenuBar: true,
    title: 'Record for Hermes', webPreferences: { preload: path.join(__dirname, 'hermes-preload.cjs'), sandbox: true, contextIsolation: true, nodeIntegration: false } });
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', event => event.preventDefault());
  window.webContents.on('page-title-updated', event => event.preventDefault());
  window.once('ready-to-show', () => { if (view.code !== 'occupied') showWindow(); });
  ipcMain.handle('apparent:hermes-read', event => { sender(event); return view; });
  ipcMain.handle('apparent:hermes-start', async event => {
    sender(event);
    if (view.phase !== 'confirm' || running) return view;
    running = true; view = { phase: 'starting' };
    try {
      await mkdir(app.getPath('userData'), { recursive: true, mode: 0o700 });
      const journal = await open(path.join(app.getPath('userData'), `${request.token}.json`), 'wx', 0o600);
      try { await journal.writeFile(JSON.stringify({ created: Date.now() })); } finally { await journal.close(); }
      try { await ready(); }
      catch {
        const child = spawn(executable, ['launch'], { detached: true, stdio: 'ignore' });
        await new Promise<void>((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
        child.unref();
        for (let attempt = 0; ; attempt++) {
          try { await ready(); break; }
          catch { if (attempt === 9) throw new Error('Open Apparent and complete onboarding, then try again.'); await new Promise(resolve => setTimeout(resolve, 500)); }
        }
      }
      window.hide();
      // Hiding only the window leaves Electron active on macOS, which records
      // the confirmation as the capture's app instead of the previous app.
      if (process.platform === 'darwin') app.hide();
      await owned.start();
      while (!window.isDestroyed()) {
        const receipt = await owned.inspect();
        if (receipt.status === 'saved') { view = { phase: 'saved', source: await owned.source() }; showWindow(); break; }
        if (receipt.status === 'cancelled') { view = { phase: 'cancelled' }; showWindow(); break; }
        if (!['starting', 'recording', 'processing'].includes(receipt.status ?? '')) throw new Error('Recording did not start. Check Apparent’s permissions and active capture.');
        view = { phase: receipt.status as 'starting' | 'recording' | 'processing' };
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    } catch (error) { view = { phase: 'error', message: error instanceof Error ? error.message : 'Unable to record' }; showWindow(); }
    finally { running = false; }
    return view;
  });
  // A URL must not put a new window into an existing capture. Admission is
  // checked atomically again by the native engine if the person confirms.
  const occupied = await new Promise<boolean>(resolve => execFile(executable, ['capture', 'state'], { timeout: 4500, maxBuffer: 65536, encoding: 'utf8' }, (_error, stdout) => {
    try { resolve(['selecting', 'starting', 'recording', 'processing'].includes(JSON.parse(stdout)?.state?.phase)); } catch { resolve(false); }
  }));
  if (occupied) view = { phase: 'error', code: 'occupied', message: 'Finish the current Apparent capture before recording from Hermes.' };
  await window.loadFile(path.join(__dirname, 'index.html'), { query: { surface: 'hermes' } });
}).catch(() => app.quit());
app.on('window-all-closed', () => app.quit());

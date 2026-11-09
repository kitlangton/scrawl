import {app, BrowserWindow, ipcMain, session} from 'electron';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {execFile} from 'node:child_process';
import {lstatSync} from 'node:fs';
const adapter = process.argv.find(value => value.startsWith('--installer-adapter='))?.slice('--installer-adapter='.length);
const bundledAdapter = path.resolve(__dirname, '../../../../MacOS/Installer');
if (process.platform !== 'darwin' || adapter !== bundledAdapter) throw Error('Installer requires its bundled Mac adapter');
const adapterFile = lstatSync(bundledAdapter);
if (!adapterFile.isFile() || adapterFile.isSymbolicLink()) throw Error('Installer adapter must be a regular bundled executable');
app.setName('Install apparent');
let busy = false;
app.whenReady().then(async () => {
  session.defaultSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  session.defaultSession.setPermissionCheckHandler(() => false);
  const page = pathToFileURL(path.join(__dirname, 'ui/src/installer/index.html')).href;
  const window = new BrowserWindow({width: 720, height: 560, minWidth: 560, minHeight: 520,
    title: 'Install apparent', webPreferences: {preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true, sandbox: true, nodeIntegration: false, webSecurity: true}});
  window.removeMenu();
  window.on('close', event => {if (busy) event.preventDefault();});
  window.webContents.setWindowOpenHandler(() => ({action: 'deny'}));
  window.webContents.on('will-navigate', event => event.preventDefault());
  window.webContents.on('will-attach-webview', event => event.preventDefault());
  ipcMain.handle('installer:action', async (event, request: unknown) => {
    if (event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame ||
        event.senderFrame.url !== page || !request || typeof request !== 'object') throw Error('Forbidden installer request');
    const value = request as {action?: unknown; replace?: unknown};
    if (typeof value.action !== 'string' || !['status','install','open'].includes(value.action) ||
        Object.keys(value).some(key => !['action','replace'].includes(key)) ||
        (value.replace !== undefined && typeof value.replace !== 'boolean')) throw Error('Invalid installer request');
    if (busy) throw Error('The installer is already working.');
    busy = true;
    try {
      const result = await new Promise<Record<string, unknown>>((resolve, reject) => {
        execFile(bundledAdapter, ['--request', JSON.stringify(value)], {timeout: 120000, maxBuffer: 32768}, (error, stdout) => {
          let reply: Record<string, unknown>;
          try { reply = JSON.parse(stdout); } catch { reject(Error('The installer stopped before confirming installation. Check Applications before trying again.')); return; }
          if (error || typeof reply.error === 'string') reject(Error(String(reply.error ?? 'Installation failed.')));
          else resolve(reply);
        });
      });
      if (value.action === 'open' && result.opened) {busy = false; app.quit();}
      return result;
    } finally { busy = false; }
  });
  await window.loadURL(page);
});
app.on('window-all-closed', () => app.quit());

import { app, BrowserWindow, session } from 'electron';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { registerOnboardingProgressBridge } from './onboarding-progress-bridge.cts';
let hostShutdown: Promise<void> = Promise.resolve();

// Separate development identity; never touch the installed Apparent profile.
app.setName('Apparent Foundation');
if (process.env.APPARENT_FOUNDATION_PROFILE) {
  app.setPath('userData', process.env.APPARENT_FOUNDATION_PROFILE);
}
app.whenReady().then(async () => {
  session.defaultSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  const entry = pathToFileURL(join(__dirname, 'index.html')).href;
  session.defaultSession.webRequest.onBeforeRequest((details, callback) => {
    const allowed = details.url.startsWith(pathToFileURL(__dirname + '/').href);
    callback({ cancel: !allowed });
  });
  let host: { store: import('@apparent/app-contracts/host').ProgressStore<unknown>; close(): Promise<void> } | undefined;
  try {
    const ownerBinding = await import(pathToFileURL(join(__dirname, 'owner-binding.mjs')).href) as {
      createOnboardingProgressStore?: (input: { configPath: string; configHostPath: string }) => Promise<typeof host>;
    };
    if (typeof ownerBinding.createOnboardingProgressStore === 'function') {
      try {
        host = await ownerBinding.createOnboardingProgressStore({
          configPath: join(app.getPath('userData'), 'config.json'),
          configHostPath: join(__dirname, 'native', 'apparent-config-host'),
        });
      } catch (error) { console.error('Onboarding progress host unavailable', error); }
    }
  } catch (error) { console.error('Onboarding assembly binding unavailable', error); }
  const window = new BrowserWindow({
    title: 'Apparent Foundation', width: 900, height: 620,
    webPreferences: { nodeIntegration: false, contextIsolation: true, sandbox: true, webSecurity: true,
      preload: join(__dirname, 'preload.cjs') },
  });
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', (event, url) => { if (url !== entry) event.preventDefault(); });
  const disposeProgress = host ? registerOnboardingProgressBridge({ window, entryUrl: entry, store: host.store }) : undefined;
  window.once('closed', () => {
    disposeProgress?.();
    const closing = host;
    hostShutdown = closing ? Promise.resolve().then(() => closing.close()) : Promise.resolve();
    hostShutdown.catch(error => console.error('Onboarding progress shutdown failed', error));
  });
  void window.loadURL(entry);
});
app.on('window-all-closed', () => {
  void hostShutdown.catch(() => {}).finally(() => app.quit());
});

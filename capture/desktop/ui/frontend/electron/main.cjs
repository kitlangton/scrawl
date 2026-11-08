// Presentation only: the owning Rust process admits every command.
function startControls() {
  const {app, BrowserWindow, ipcMain, Menu, session, nativeTheme} = require('electron');
  const path = require('node:path');
  const {pathToFileURL} = require('node:url');
  const readline = require('node:readline');
  const followWindowTheme = require('@apparent/ui/window-theme');
  app.setName('Apparent');
  app.setPath('userData', path.join(app.getPath('appData'), 'apparent-controls'));
  nativeTheme.themeSource = 'system';
  let window;
  let busy = false;
  let status = null;
  let painted = false;
  let mounted = false;
  let retired = false;
  const show = () => {
    if (!retired && painted && mounted) { window.show(); process.stdout.write('{"ready":true}\n'); }
  };
  process.stdin.on('end', () => { retired = true; app.quit(); });
  process.stdout.on('error', () => { retired = true; app.quit(); });
  readline.createInterface({input: process.stdin}).on('line', line => {
    if (line === 'show' && window && !window.isDestroyed()) {
      if (!retired && painted && mounted) { window.restore(); window.show(); window.focus(); }
    } else if (line === 'busy' || line === 'idle') {
      busy = line === 'busy';
      if (window && !window.isDestroyed()) window.webContents.send('apparent:controls-busy', busy);
    } else if (line.startsWith('status ')) {
      try {
        const value = JSON.parse(line.slice(7));
        if (value === null || typeof value === 'string') {
          status = value;
          if (window && !window.isDestroyed()) window.webContents.send('apparent:controls-status', status);
        }
      } catch {}
    }
  });
  app.whenReady().then(() => {
    if (retired) return;
    Menu.setApplicationMenu(null);
    session.defaultSession.setPermissionRequestHandler((_contents, _permission, reply) => reply(false));
    session.defaultSession.setPermissionCheckHandler(() => false);
    window = new BrowserWindow({width: 420, height: 500, show: false,
      title: 'Apparent', backgroundColor: followWindowTheme.backgroundColor(nativeTheme),
      webPreferences: {preload: path.join(__dirname, 'preload.cjs'), additionalArguments: ['--controls'],
        contextIsolation: true, nodeIntegration: false, sandbox: true, webSecurity: true}});
    followWindowTheme(nativeTheme, window);
    const page = pathToFileURL(path.join(__dirname, '../dist/src/controls/index.html')).href;
    window.webContents.setWindowOpenHandler(() => ({action: 'deny'}));
    window.webContents.on('will-navigate', event => event.preventDefault());
    window.webContents.on('will-attach-webview', event => event.preventDefault());
    ipcMain.handle('apparent:controls-mounted', (event, ...args) => {
      if (event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame ||
          event.senderFrame.url !== page || args.length !== 0) throw new Error('Forbidden controls request');
      mounted = true; show(); return {busy, status};
    });
    ipcMain.handle('apparent:controls-command', (event, ...args) => {
      if (window.isDestroyed() || event.sender !== window.webContents ||
          event.senderFrame !== window.webContents.mainFrame || event.senderFrame.url !== page ||
          args.length !== 1 || !['record', 'choose', 'latest', 'settings'].includes(args[0])) {
        throw new Error('Forbidden controls request');
      }
      if (!retired && !busy) process.stdout.write(JSON.stringify({action: args[0]}) + '\n');
    });
    window.webContents.on('render-process-gone', () => app.exit(1));
    window.webContents.on('did-fail-load', () => app.exit(1));
    window.once('ready-to-show', () => {
      painted = true; show();
    });
    window.loadURL(page);
  });
  app.on('window-all-closed', () => app.quit());
}

if(process.argv.includes('--controls')) { startControls(); }
else if(process.argv.includes('--onboarding')) { require('./onboarding.cjs'); } else {
const { app, BrowserWindow, dialog, ipcMain, Menu, session, shell, nativeTheme } = require('electron');
const os = require('node:os');
let exitWithReports = code => app.exit(code);
if (process.env.APPARENT_DESKTOP_REPORTING === '1') {
  exitWithReports = require('../dist/assets/reporting.cjs').install(app, kind => require('./runtime.cjs').reportException(os.homedir(), kind));
}
const {PublicShare} = require('./public-share.cjs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { readRuntimeState, storageLocation, errorReporting, captureReference, storagePreference, microphoneEnabled } = require('./runtime.cjs');
const { locationCandidate, resolveLocationCandidate, normalizeSelectedPath } = require('./storage-policy.cjs');
const { ConnectSettings, actions } = require('./connect.cjs');
const followWindowTheme = require('@apparent/ui/window-theme');
nativeTheme.themeSource = 'system';
const settings = process.argv.includes('--settings-storage');
const about = settings && process.argv.includes('--settings-about');
let aboutState;

// Deliberately separate from the installed application's profile and identity.
app.setName(settings ? 'Apparent Settings' : 'Apparent UI Foundation');
app.setPath('userData', path.join(app.getPath('appData'), settings ? 'apparent-settings' : 'apparent-ui-foundation'));

app.whenReady().then(() => {
  if (settings) {
    Menu.setApplicationMenu(null);
    if (process.platform === 'darwin') app.setActivationPolicy('accessory');
  }
  session.defaultSession.setPermissionRequestHandler((_contents, _permission, reply) => reply(false));
  session.defaultSession.setPermissionCheckHandler(() => false);
  const window = new BrowserWindow({
    show: !settings,
    // The preload needs a render opportunity before the first presentation.
    ...(settings ? { paintWhenInitiallyHidden: true } : {}),
    width: settings ? 720 : 920, height: settings ? 530 : 760,
    minWidth: 640, minHeight: settings ? 530 : 620,
    ...(settings && process.platform === 'darwin' ? {
      titleBarStyle: 'hiddenInset', resizable: false, fullscreenable: false,
      acceptFirstMouse: true,
    } : {}),
    title: settings ? 'Apparent Settings — Storage' : 'Apparent — UI foundation',
    backgroundColor: followWindowTheme.backgroundColor(nativeTheme),
    webPreferences: {
      preload: path.join(__dirname, settings ? 'storage-preload.cjs' : 'preload.cjs'),
      contextIsolation: true, nodeIntegration: false, sandbox: true,
      webSecurity: true, spellcheck: false,
    },
  });
  // A parent show must not expose Chromium's empty initial document.
  let settingsPainted = false;
  let settingsMounted = false;
  let settingsClosing = false;
  let settingsFailed = false;
  let startupTimer;
  const failSettings = () => {
    settingsFailed = settingsClosing = true;
    clearTimeout(startupTimer);
    exitWithReports(1);
  };
  const showSettings = () => {
    if (!settingsPainted || !settingsMounted || settingsClosing || window.isDestroyed()) return;
    clearTimeout(startupTimer);
    window.show();
    window.focus();
  };
  if (settings) {
    // First paint alone can be an empty document with a missing entry module.
    // Require the React surface too, or release the parent into native fallback.
    startupTimer = setTimeout(failSettings, 15000);
    startupTimer.unref();
    window.once('ready-to-show', () => {
      settingsPainted = true;
      showSettings();
    });
  }
  // AppKit applies the inset titlebar metrics after constructor-time centering.
  if (settings && process.platform === 'darwin') window.center();
  followWindowTheme(nativeTheme, window);
  window.removeMenu();
  const page = pathToFileURL(path.join(__dirname, settings ? '../dist/settings.html' : '../dist/index.html')).href;
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', (event) => event.preventDefault());
  window.webContents.on('will-attach-webview', (event) => event.preventDefault());
  let pending;
  let lastRead = 0;
  let lastResult = { status: 'disconnected' };
  const validate = (event, args) => {
    if (window.isDestroyed() || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame ||
        event.senderFrame.url !== (about ? page + '#about' : page) || args.length !== 0) throw new Error('Forbidden bridge request');
  };
  const validateFoundation = (event, args) => {
    if (settings) throw new Error('Unavailable in Storage Settings');
    validate(event, args);
  };
  ipcMain.handle('apparent:settings-mounted', (event, ...args) => {
    validate(event, args);
    if (!settings) throw new Error('Unavailable outside Settings');
    settingsMounted = true;
    showSettings();
  });
  ipcMain.handle('apparent:settings-frame-ready', (event, ...args) => {
    validate(event, args);
    if (!settings) throw new Error('Unavailable outside Settings');
    settingsPainted = true;
    showSettings();
  });
  ipcMain.handle('apparent:about-link', (event, link, ...args) => {
    validate(event, args);
    if (!about || !['website', 'source'].includes(link)) throw new Error('Forbidden link');
    return shell.openExternal(link === 'website' ? 'https://app.tryapparent.com' : 'https://github.com/tryapparent/apparent');
  });
  ipcMain.handle('apparent:about-read', (event, ...args) => {
    validate(event, args);
    if (!about) throw new Error('Unavailable outside About');
    return aboutState;
  });
  ipcMain.handle('apparent:about-action', (event, action, ...args) => {
    validate(event, args);
    if (!about || !action || typeof action !== 'object') throw new Error('Forbidden About action');
    const simple = ['check-for-updates', 'dismiss-update-check', 'update-now', 'update-later', 'tour'];
    if (!(Object.keys(action).length === 1 && simple.includes(action.action)) &&
        !(Object.keys(action).length === 3 && action.action === 'set' && action.key === 'updateChannel' && ['stable', 'main'].includes(action.value)))
      throw new Error('Forbidden About action');
    process.stdout.write(`${JSON.stringify(action)}\n`);
  });
  ipcMain.handle('apparent:navigate-settings', (event, ...args) => {
    validate(event, []);
    const destinations = about ? ['general', 'recording', 'permissions', 'agents', 'storage'] : ['general', 'recording', 'permissions', 'agents', 'about'];
    if (!settings || args.length !== 1 || !destinations.includes(args[0])) throw new Error('Forbidden Settings page');
    process.stdout.write(`${JSON.stringify({ action: 'navigate', pane: args[0] })}\n`);
  });
  ipcMain.handle('apparent:show-capture-folder', (event, ...args) => {
    validate(event, args);
    if (!settings) throw new Error('Unavailable outside Settings');
    process.stdout.write(`${JSON.stringify({ action: 'show-captures' })}\n`);
  });
  if (settings) {
    const closeSettings = () => { settingsClosing = true; clearTimeout(startupTimer); app.quit(); };
    // A parent can exit before Chromium closes its child. Retire quietly when
    // its output pipe breaks instead of opening Electron's error dialog.
    process.stdout.on('error', error => { if (error.code === 'EPIPE') closeSettings(); else failSettings(); });
    // Only the Rust Settings owner supplies this private pipe. EOF closes its child.
    if (process.argv.includes('--settings-parent')) {
    const lines = require('node:readline').createInterface({ input: process.stdin });
    lines.on('line', line => {
      try {
        const command = JSON.parse(line);
        if (command.type === 'about-state' && about) {
          aboutState = command;
          window.webContents.send('apparent:about-state', command);
        }
        if (command.type === 'close') closeSettings();
        if (command.type === 'show') showSettings();
      } catch { /* malformed parent message has no effect */ }
    });
    lines.on('close', closeSettings);
    }
    window.on('close', () => {
      const userDismissed = !settingsClosing && !settingsFailed;
      settingsClosing = true;
      clearTimeout(startupTimer);
      // Failure is not user dismissal: Rust must be allowed to restore Storage.
      if (userDismissed) process.stdout.write(`${JSON.stringify({ action: 'close' })}\n`);
    });
    window.webContents.on('did-fail-load', failSettings);
    window.webContents.on('render-process-gone', failSettings);
  }
  let generation = 0;
  let mutation = false;
  let storageVersion = 0;
  const connect = new ConnectSettings(os.homedir());
  let pendingConnect;
  const publicShare = new PublicShare(os.homedir());
  let reference;
  let referencePending;
  let retentionRevision;
  let retentionRead;
  const invalidate = () => { generation += 1; storageVersion += 1; reference = undefined; retentionRevision = undefined; connect.close(); publicShare.close(); };
  window.webContents.on('did-start-navigation', (_event, _url, _inPlace, isMainFrame) => { if (isMainFrame) invalidate(); });
  window.webContents.on('will-navigate', invalidate);
  window.webContents.on('render-process-gone', invalidate);
  window.on('closed', invalidate);
  const requestConnect = (action) => {
    if (pendingConnect) return action === undefined ? pendingConnect : Promise.resolve({ status: 'error' });
    const token = generation;
    const request = connect.request(action).then(result => {
      if (token !== generation || window.isDestroyed()) return { status: 'cancelled' };
      // Only this explicit zero-argument action may open the URI just authorized
      // by the current Rust controller. Never accept a renderer-supplied URL.
      if (result.openUri !== undefined) {
        void shell.openExternal(result.openUri).catch(() => {});
      }
      const { openUri: _openUri, ...projected } = result;
      return projected;
    }).catch(() => ({ status: 'error' })).finally(() => {
      if (pendingConnect === request) pendingConnect = undefined;
    });
    pendingConnect = request;
    return request;
  };
  ipcMain.handle('apparent:read-connect-settings', (event, ...args) => {
    validateFoundation(event, args);
    return requestConnect();
  });
  for (const action of actions) {
    ipcMain.handle(`apparent:connect-${action}`, (event, ...args) => {
      validateFoundation(event, args);
      return requestConnect(action);
    });
  }
  const resolveLocation = async (facts) => {
    const candidate = locationCandidate(facts);
    if (candidate.snapshot) return { status: 'connected', revision: candidate.revision, storage: candidate.snapshot };
    const probe = await storageLocation(os.homedir(), 'probe', { path: candidate.probePath, expected: candidate.revision });
    if (probe.status !== 'connected') return probe;
    return { status: 'connected', revision: candidate.revision,
      storage: resolveLocationCandidate(facts, probe.available) };
  };
  const readLocation = async () => {
    const result = await storageLocation(os.homedir(), 'facts');
    if (result.status !== 'connected') return result;
    try { return await resolveLocation(result.location); }
    catch { return { status: 'error', code: 'invalid_response' }; }
  };
  const project = (result) => {
    if (result.status !== 'connected') return result;
    return { status: 'connected', storage: result.storage };
  };
  let reportingMutation = false;
  let microphoneMutation = false;
  ipcMain.handle('apparent:read-microphone-enabled', (event, ...args) => { validateFoundation(event, args); return microphoneEnabled(os.homedir()); });
  ipcMain.handle('apparent:set-microphone-enabled', async (event, enabled, ...args) => {
    validateFoundation(event, args);
    if (typeof enabled !== 'boolean') throw new Error('Forbidden microphone preference');
    if (microphoneMutation) return { status: 'error', code: 'app_busy' };
    microphoneMutation = true;
    try { return await microphoneEnabled(os.homedir(), enabled); }
    finally { microphoneMutation = false; }
  });
  ipcMain.handle('apparent:read-error-reporting', async (event, ...args) => {
    validateFoundation(event, args);
    return errorReporting(os.homedir());
  });
  ipcMain.handle('apparent:set-error-reporting', async (event, enabled, ...args) => {
    validateFoundation(event, args);
    if (args.length || typeof enabled !== 'boolean') throw new Error('Forbidden bridge request');
    if (reportingMutation) return { status: 'error', code: 'app_busy' };
    reportingMutation = true;
    try { return await errorReporting(os.homedir(), enabled); }
    finally { reportingMutation = false; }
  });
  ipcMain.handle('apparent:read-capture-location', async (event, ...args) => {
    validate(event, args);
    const version = storageVersion;
    const result = await readLocation();
    return version === storageVersion ? project(result) : { status: 'cancelled' };
  });
  for (const operation of ['change', 'reset']) {
    ipcMain.handle(`apparent:${operation}-capture-location`, async (event, ...args) => {
      validate(event, args);
      if (mutation) return { status: 'error', code: 'app_busy' };
      mutation = true;
      storageVersion += 1;
      const token = generation;
      try {
        const before = await readLocation();
        if (token !== generation) return { status: 'cancelled' };
        if (before.status !== 'connected') return project(before);
        const fields = { expected: before.revision };
        if (operation === 'change') {
          const selected = await dialog.showOpenDialog(window, {
            title: 'Choose capture location', buttonLabel: 'Choose',
            properties: ['openDirectory', 'createDirectory'],
            ...(path.isAbsolute(before.storage.effectivePath) ? { defaultPath: path.dirname(before.storage.effectivePath) } : {}),
            message: 'New captures use this location. Existing captures stay where they are.',
          });
          if (token !== generation || window.isDestroyed() || selected.canceled) return { status: 'cancelled' };
          if (selected.filePaths.length !== 1) return { status: 'error', code: 'invalid_path' };
          fields.path = normalizeSelectedPath(selected.filePaths[0]);
          if (!fields.path) return { status: 'error', code: 'invalid_path' };
        }
        if (token !== generation) return { status: 'cancelled' };
        const saved = await storageLocation(os.homedir(), operation === 'change' ? 'set' : 'reset', fields);
        if (saved.status !== 'connected') return project(saved);
        if (token !== generation || window.isDestroyed()) return { status: 'cancelled' };
        return project(await resolveLocation(saved.location));
      } catch {
        return { status: 'error', code: 'bridge_failed' };
      } finally {
        storageVersion += 1;
        mutation = false;
      }
    });
  }
  for (const operation of ['read', 'copy']) {
    ipcMain.handle(`apparent:${operation}-capture-reference`, async (event, ...args) => {
      validateFoundation(event, args);
      // Coalesce StrictMode/read refreshes, but never repeat or overlap a copy.
      if (referencePending) return operation === 'read' && referencePending.operation === 'read'
        ? referencePending.promise : { status: 'error', code: 'app_busy' };
      const expected = reference?.status === 'connected' ? reference.id : null;
      if (operation === 'copy' && expected === null) return { status: 'error', code: 'refresh_required' };
      const token = generation;
      reference = undefined;
      const promise = captureReference(os.homedir(), operation, expected).then((result) => {
        if (token !== generation) return { status: 'cancelled' };
        if (result.status === 'connected') reference = result;
        return result;
      }).catch(() => ({ status: 'error', code: 'bridge_failed' })).finally(() => { referencePending = undefined; });
      referencePending = { operation, promise };
      return promise;
    });
  }
  for (const action of ['start','read','cancel']) {
    ipcMain.handle(`apparent:${action}-public-share`, async (event,...args) => {
      validateFoundation(event,args);
      return publicShare.request(action, reference?.status==='connected'?reference.id:null);
    });
  }
  const projectRetention = (result) => {
    if (result.status !== 'connected') return result;
    retentionRevision = result.revision;
    const { revision: _revision, ...safe } = result;
    return safe;
  };
  ipcMain.handle('apparent:read-retention', async (event, ...args) => {
    validate(event, args);
    const version = storageVersion;
    if (!retentionRead || retentionRead.version !== version) {
      const pending = { version, promise: storagePreference(os.homedir(), 'retention') };
      retentionRead = pending;
      void pending.promise.finally(() => { if (retentionRead === pending) retentionRead = undefined; });
    }
    const result = await retentionRead.promise;
    if (version !== storageVersion) return { status: 'cancelled' };
    retentionRevision = undefined;
    return projectRetention(result);
  });
  ipcMain.handle('apparent:read-capture-usage', async (event, ...args) => {
    validate(event, args);
    const version = storageVersion;
    const result = await storagePreference(os.homedir(), 'usage');
    return version === storageVersion ? result : { status: 'cancelled' };
  });
  for (const operation of ['save', 'reset']) {
    ipcMain.handle(`apparent:${operation}-retention`, async (event, ...args) => {
      const policy = operation === 'save' ? args[0] : undefined;
      validate(event, operation === 'save' && args.length === 1 ? [] : args);
      if (operation === 'save' && (!policy || Object.keys(policy).length !== 2 ||
          !['mediaDays', 'bundleDays'].every(key => Object.hasOwn(policy, key)) ||
          !['mediaDays', 'bundleDays'].every(key => policy[key] === null ||
            (Number.isInteger(policy[key]) && policy[key] >= 0 && policy[key] <= 0xffffffff)))) {
        throw new Error('Invalid retention policy');
      }
      if (mutation) return { status: 'error', code: 'app_busy' };
      if (!retentionRevision) return { status: 'error', code: 'refresh_required' };
      const expected = retentionRevision;
      retentionRevision = undefined;
      mutation = true;
      storageVersion += 1;
      const token = generation;
      try {
        const result = await storagePreference(os.homedir(), `${operation}retention`, { expected, policy });
        return token === generation ? projectRetention(result) : { status: 'cancelled' };
      } finally { storageVersion += 1; mutation = false; }
    });
  }
  ipcMain.handle('apparent:read-runtime-state', (event, ...args) => {
    validateFoundation(event, args);
    // One bounded request, only on load or manual refresh; no recording-time polling.
    if (pending) return pending;
    if (Date.now() - lastRead < 1000) return lastResult;
    pending = readRuntimeState(os.homedir()).then((result) => {
      lastResult = result;
      lastRead = Date.now();
      return result;
    }).finally(() => { pending = undefined; });
    return pending;
  });
  void window.loadURL(about ? page + '#about' : page);
});
app.on('window-all-closed', () => app.quit());

}

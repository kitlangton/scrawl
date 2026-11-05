require('./runtime.cjs').installBuildIdentity(require('electron'));
require('./runtime.cjs').installExceptionReporting(require('electron'));
if(process.argv.includes('--onboarding')) { require('./onboarding.cjs'); } else {
const { app, BrowserWindow, dialog, ipcMain, Menu, session, shell, nativeTheme } = require('electron');
const os = require('node:os');
const {PublicShare} = require('./public-share.cjs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { drawing, readRuntimeState, storageLocation, errorReporting, captureReference, storagePreference, microphoneEnabled } = require('./runtime.cjs');
const { locationCandidate, resolveLocationCandidate, normalizeSelectedPath } = require('./storage-policy.cjs');
const { ConnectSettings, actions } = require('./connect.cjs');
const followWindowTheme = require('@apparent/ui/window-theme');
nativeTheme.themeSource = 'system';
const connectSettings = process.argv.includes('--settings-connect');
const settings = connectSettings || process.argv.includes('--settings-storage');
const initialConnectAction = process.argv.find(value => value.startsWith('--connect-action='))?.slice('--connect-action='.length);

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
    // Settings must keep rendering while hidden so the renderer can confirm a
    // committed frame before the first native presentation.
    ...(settings ? { paintWhenInitiallyHidden: true } : {}),
    width: settings ? 720 : 920, height: settings ? 530 : 760,
    minWidth: 640, minHeight: settings ? 530 : 620,
    ...(settings && process.platform === 'darwin' ? {
      titleBarStyle: 'hiddenInset', resizable: false, fullscreenable: false,
      acceptFirstMouse: true,
    } : {}),
    title: connectSettings ? 'Apparent Settings — Connect' : settings ? 'Apparent Settings — Storage' : 'Apparent — UI foundation',
    backgroundColor: followWindowTheme.backgroundColor(nativeTheme),
    webPreferences: {
      preload: path.join(__dirname, connectSettings ? 'preloads/connect.cjs' : settings ? 'preloads/storage.cjs' : 'preload.cjs'),
      contextIsolation: true, nodeIntegration: false, sandbox: true,
      webSecurity: true, spellcheck: false,
    },
  });
  // A parent show can arrive before Chromium paints the Settings page. Keep
  // the empty initial document hidden through that request too.
  let settingsPainted = false;
  let settingsMounted = false;
  let settingsClosing = false;
  let settingsFailed = false;
  let startupTimer;
  const failSettings = () => {
    settingsFailed = settingsClosing = true;
    clearTimeout(startupTimer);
    app.exit(1);
  };
  const showSettings = () => {
    if (!settingsPainted || !settingsMounted || settingsClosing || window.isDestroyed()) return;
    clearTimeout(startupTimer);
    window.show();
    window.focus();
  };
  if (settings) {
    // Chromium can paint an empty document when an entry module fails to load.
    // Require the real React surface too, or let the parent restore native UI.
    startupTimer = setTimeout(failSettings, 15000);
    startupTimer.unref();
    window.once('ready-to-show', () => {
      settingsPainted = true;
      showSettings();
    });
  }
  // Recenter after AppKit applies the inset titlebar metrics. Constructor-time
  // centering places this frame one titlebar lower than the native Settings.
  if (settings && process.platform === 'darwin') window.center();
  followWindowTheme(nativeTheme, window);
  window.removeMenu();
  const pageUrl = pathToFileURL(path.join(__dirname, settings ? '../dist/settings.html' : '../dist/index.html'));
  if (connectSettings) {
    pageUrl.searchParams.set('surface', 'connect');
  }
  const page = pageUrl.href;
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', (event) => event.preventDefault());
  window.webContents.on('will-attach-webview', (event) => event.preventDefault());
  let pending;
  let lastRead = 0;
  let lastResult = { status: 'disconnected' };
  const validate = (event, args) => {
    if (window.isDestroyed() || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame ||
        event.senderFrame.url !== page || args.length !== 0) throw new Error('Forbidden bridge request');
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
  let drawingRevision;
  const validateStorage = (event, args) => {
    if (connectSettings) throw new Error('Unavailable in Connect Settings');
    validate(event, args);
  };
  const validateConnect = (event, args) => {
    if (settings && !connectSettings) throw new Error('Unavailable in Storage Settings');
    validate(event, args);
  };
  const validateFoundation = (event, args) => {
    if (settings) throw new Error('Unavailable in Storage Settings');
    validate(event, args);
  };
  ipcMain.handle('apparent:navigate-settings', (event, ...args) => {
    validate(event, []);
    if (!settings || args.length !== 1 || !['general', 'recording', 'permissions', 'agents', 'about', ...(connectSettings ? ['storage'] : [])].includes(args[0])) throw new Error('Forbidden Settings page');
    process.stdout.write(`${JSON.stringify({ action: 'navigate', pane: args[0] })}\n`);
  });
  ipcMain.handle('apparent:show-capture-folder', (event, ...args) => {
    validateStorage(event, args);
    if (!settings) throw new Error('Unavailable outside Settings');
    process.stdout.write(`${JSON.stringify({ action: 'show-captures' })}\n`);
  });
  if (settings) {
    const closeSettings = () => { settingsClosing = true; clearTimeout(startupTimer); app.quit(); };
    // Only the Rust Settings owner supplies this private pipe. EOF closes its child.
    if (process.argv.includes('--settings-parent')) {
    const lines = require('node:readline').createInterface({ input: process.stdin });
    lines.on('line', line => {
      try {
        const command = JSON.parse(line);
        if (command.type === 'close') closeSettings();
        if (command.type === 'show') showSettings();
      } catch { /* malformed parent message has no effect */ }
    });
    lines.on('close', closeSettings);
    }
    window.on('close', () => {
      settingsClosing = true;
      clearTimeout(startupTimer);
      // A failed startup must reach the parent's Exited fallback, not look
      // like a user dismissing Settings.
      if (!settingsFailed) process.stdout.write(`${JSON.stringify({ action: 'close' })}\n`);
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
  const invalidate = () => { generation += 1; drawingRevision = undefined; storageVersion += 1; reference = undefined; retentionRevision = undefined; connect.close(); publicShare.close(); };
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
  let openingConnectAction = connectSettings && actions.has(initialConnectAction) ? initialConnectAction : undefined;
  ipcMain.handle('apparent:read-connect-settings', async (event, ...args) => {
    validateConnect(event, args);
    // Consume the native user action once. Renderer reload must never replay it.
    const action = openingConnectAction;
    openingConnectAction = undefined;
    const token = generation;
    const result = await requestConnect();
    if (action === undefined || result.status !== 'connected' || token !== generation) return result;
    return requestConnect(action);
  });
  for (const action of actions) {
    ipcMain.handle(`apparent:connect-${action}`, (event, ...args) => {
      validateConnect(event, args);
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
    validateStorage(event, args);
    const version = storageVersion;
    const result = await readLocation();
    return version === storageVersion ? project(result) : { status: 'cancelled' };
  });
  for (const operation of ['change', 'reset']) {
    ipcMain.handle(`apparent:${operation}-capture-location`, async (event, ...args) => {
      validateStorage(event, args);
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
    validateStorage(event, args);
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
    validateStorage(event, args);
    const version = storageVersion;
    const result = await storagePreference(os.homedir(), 'usage');
    return version === storageVersion ? result : { status: 'cancelled' };
  });
  for (const operation of ['save', 'reset']) {
    ipcMain.handle(`apparent:${operation}-retention`, async (event, ...args) => {
      const policy = operation === 'save' ? args[0] : undefined;
      validateStorage(event, operation === 'save' && args.length === 1 ? [] : args);
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
  let drawingPending = false;
  for (const operation of ['read', 'undo']) {
    ipcMain.handle(`apparent:${operation}-drawing`, async (event, ...args) => {
      validateFoundation(event, args);
      if (drawingPending) return { status: 'error', code: 'app_busy' };
      if (operation === 'undo' && !drawingRevision) return { status: 'error', code: 'refresh_required' };
      const expected = drawingRevision;
      drawingRevision = undefined;
      drawingPending = true;
      const token = generation;
      try {
        const result = await drawing(os.homedir(), operation, expected);
        if (token !== generation) return { status: 'cancelled' };
        if (result.status !== 'connected') return result;
        drawingRevision = result.revision;
        return { status: 'connected', active: result.revision !== null, strokes: result.strokes };
      } finally { drawingPending = false; }
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
  void window.loadURL(page);
});
app.on('window-all-closed', () => app.quit());

}

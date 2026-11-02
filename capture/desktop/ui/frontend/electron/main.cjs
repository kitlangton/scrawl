const { app, BrowserWindow, dialog, ipcMain, Menu, session, shell } = require('electron');
const os = require('node:os');
const {PublicShare} = require('./public-share.cjs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { drawing, readRuntimeState, captureLocation, errorReporting, captureReference, storagePreference } = require('./runtime.cjs');
const { ConnectSettings, actions } = require('./connect.cjs');
const settings = process.argv.includes('--settings-storage');

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
    width: 920, height: 760, minWidth: 640, minHeight: 620,
    title: settings ? 'Apparent Settings — Storage' : 'Apparent — UI foundation', backgroundColor: settings ? '#201b24' : '#faf9f6',
    webPreferences: {
      preload: path.join(__dirname, settings ? 'storage-preload.cjs' : 'preload.cjs'),
      contextIsolation: true, nodeIntegration: false, sandbox: true,
      webSecurity: true, spellcheck: false,
    },
  });
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
        event.senderFrame.url !== page || args.length !== 0) throw new Error('Forbidden bridge request');
  };
  let drawingRevision;
  const validateFoundation = (event, args) => {
    if (settings) throw new Error('Unavailable in Storage Settings');
    validate(event, args);
  };
  ipcMain.handle('apparent:navigate-settings', (event, ...args) => {
    validate(event, []);
    if (!settings || args.length !== 1 || !['general', 'recording', 'permissions', 'agents', 'about'].includes(args[0])) throw new Error('Forbidden Settings page');
    process.stdout.write(`${JSON.stringify({ action: 'navigate', pane: args[0] })}\n`);
  });
  ipcMain.handle('apparent:show-capture-folder', (event, ...args) => {
    validate(event, args);
    if (!settings) throw new Error('Unavailable outside Settings');
    process.stdout.write(`${JSON.stringify({ action: 'show-captures' })}\n`);
  });
  if (settings) {
    // Only the Rust Settings owner supplies this private pipe. EOF closes its child.
    if (process.argv.includes('--settings-parent')) {
    const lines = require('node:readline').createInterface({ input: process.stdin });
    lines.on('line', line => {
      try {
        const command = JSON.parse(line);
        if (command.type === 'close') app.quit();
        if (command.type === 'show') { window.show(); window.focus(); }
      } catch { /* malformed parent message has no effect */ }
    });
    lines.on('close', () => app.quit());
    }
    window.on('close', () => process.stdout.write(`${JSON.stringify({ action: 'close' })}\n`));
    window.webContents.on('did-fail-load', () => app.exit(1));
    window.webContents.on('render-process-gone', () => app.exit(1));
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
  const project = (result) => {
    if (result.status !== 'connected') return result;
    const { revision: _revision, ...storage } = result.storage;
    return { status: 'connected', storage };
  };
  let reportingMutation = false;
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
    const result = await captureLocation(os.homedir());
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
        const before = await captureLocation(os.homedir());
        if (token !== generation) return { status: 'cancelled' };
        if (before.status !== 'connected') return project(before);
        const fields = { expected: before.storage.revision };
        if (operation === 'change') {
          const selected = await dialog.showOpenDialog(window, {
            title: 'Choose capture location', buttonLabel: 'Choose',
            properties: ['openDirectory', 'createDirectory'],
            ...(path.isAbsolute(before.storage.effectivePath) ? { defaultPath: path.dirname(before.storage.effectivePath) } : {}),
            message: 'New captures use this location. Existing captures stay where they are.',
          });
          if (token !== generation || window.isDestroyed() || selected.canceled) return { status: 'cancelled' };
          if (selected.filePaths.length !== 1) return { status: 'error', code: 'invalid_path' };
          fields.path = selected.filePaths[0];
        }
        if (token !== generation) return { status: 'cancelled' };
        return project(await captureLocation(os.homedir(), operation === 'change' ? 'set' : 'reset', fields));
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
      const preset = operation === 'save' ? args[0] : undefined;
      validate(event, operation === 'save' && args.length === 1 && ['week', 'month', 'forever'].includes(preset) ? [] : args);
      if (operation === 'save' && args.length !== 1) throw new Error('Invalid preset');
      if (mutation) return { status: 'error', code: 'app_busy' };
      if (!retentionRevision) return { status: 'error', code: 'refresh_required' };
      const expected = retentionRevision;
      retentionRevision = undefined;
      mutation = true;
      storageVersion += 1;
      const token = generation;
      try {
        const result = await storagePreference(os.homedir(), `${operation}retention`, { expected, preset });
        return token === generation ? projectRetention(result) : { status: 'cancelled' };
      } finally { storageVersion += 1; mutation = false; }
    });
  }
  let drawingPending = false;
  for (const operation of ['read', 'undo']) {
    ipcMain.handle(`apparent:${operation}-drawing`, async (event, ...args) => {
      validate(event, args);
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

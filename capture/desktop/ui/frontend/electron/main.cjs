const { app, BrowserWindow, dialog, ipcMain, session } = require('electron');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { readRuntimeState, captureLocation, captureReference, storagePreference } = require('./runtime.cjs');

// Deliberately separate from the installed application's profile and identity.
app.setName('Apparent UI Foundation');
app.setPath('userData', path.join(app.getPath('appData'), 'apparent-ui-foundation'));

app.whenReady().then(() => {
  session.defaultSession.setPermissionRequestHandler((_contents, _permission, reply) => reply(false));
  session.defaultSession.setPermissionCheckHandler(() => false);
  const window = new BrowserWindow({
    width: 920, height: 760, minWidth: 640, minHeight: 620,
    title: 'Apparent — UI foundation', backgroundColor: '#faf9f6',
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true, nodeIntegration: false, sandbox: true,
      webSecurity: true, spellcheck: false,
    },
  });
  window.removeMenu();
  const page = pathToFileURL(path.join(__dirname, '../dist/index.html')).href;
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
  let generation = 0;
  let mutation = false;
  let storageVersion = 0;
  let reference;
  let referencePending;
  let retentionRevision;
  let retentionRead;
  const invalidate = () => { generation += 1; storageVersion += 1; reference = undefined; retentionRevision = undefined; };
  window.webContents.on('did-start-navigation', (_event, _url, _inPlace, isMainFrame) => { if (isMainFrame) invalidate(); });
  window.webContents.on('will-navigate', invalidate);
  window.webContents.on('render-process-gone', invalidate);
  window.on('closed', invalidate);
  const project = (result) => {
    if (result.status !== 'connected') return result;
    const { revision: _revision, ...storage } = result.storage;
    return { status: 'connected', storage };
  };
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
      validate(event, args);
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
  ipcMain.handle('apparent:read-runtime-state', (event, ...args) => {
    validate(event, args);
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

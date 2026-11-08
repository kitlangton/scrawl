// Only Rust's private inherited stdin supplies capture state. Never opens a socket or backend.
const { app, BrowserWindow, ipcMain, screen, session, nativeTheme } = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const suppliedProfile = process.argv.find(v => v.startsWith('--notes-profile='))?.slice('--notes-profile='.length);
const profile = suppliedProfile || fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'apparent-note-ui-'));
const { pathToFileURL } = require('node:url');
const readline = require('node:readline');
const { validCommand, validSnapshot, popupBounds, badgeBounds } = require('./protocol.cjs');
app.setName('Apparent Notes');
nativeTheme.themeSource = 'system';
app.setPath('userData', profile);
// Production Rust deletes its owned profile after reaping this process. Standalone
// renderer tests own their fallback; cleanup errors must never open an error dialog.
app.on('quit', () => {
  if (!suppliedProfile) {
    try { fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 25 }); }
    catch { process.stderr.write('Could not clean standalone notes UI profile\n'); }
  }
});
const frame = process.argv.find(v => v.startsWith('--capture-frame='))?.slice('--capture-frame='.length).split(',').map(Number);
if (!frame || frame.length !== 4 || !frame.every(Number.isFinite)) app.exit(1);
let latest, popup, badge;
let finishing = false, finished = false;
const send = (value) => process.stdout.write(JSON.stringify(value) + '\n');
app.whenReady().then(() => {
  if (process.platform === 'darwin') app.setActivationPolicy('accessory');
  session.defaultSession.setPermissionRequestHandler((_wc, _permission, reply) => reply(false));
  session.defaultSession.setPermissionCheckHandler(() => false);
  const page = pathToFileURL(path.join(__dirname, '../../dist/index.html')).href;
  function window(surface, width, height) {
    const win = new BrowserWindow({ title: 'Apparent notes', width, height, show: false, frame: false, transparent: true,
      resizable: false, skipTaskbar: true, alwaysOnTop: true, hasShadow: false,
      webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true,
        nodeIntegration: false, sandbox: true, spellcheck: false, webSecurity: true } });
    win.webContents.on('page-title-updated', event => event.preventDefault());
    win.removeMenu(); win.setAlwaysOnTop(true, 'floating');
    win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
    win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    win.webContents.on('will-navigate', e => e.preventDefault());
    win.webContents.on('will-attach-webview', e => e.preventDefault());
    win.webContents.on('render-process-gone', () => app.quit());
    win.loadURL(page + '#notes-' + surface);
    return win;
  }
  popup = window('popup', 332, 200); badge = window('badge', 150, 44);
  const display = screen.getDisplayNearestPoint({ x: Math.round(frame[0] + frame[2] / 2), y: Math.round(frame[1] + frame[3] / 2) });
  const area = display.workArea;
  badge.setPosition(Math.min(area.x + area.width - 156, Math.round(frame[0] + frame[2] / 2 + 150)), area.y + 12);
  const placed = new Map();
  const requested = new Map();
  function setRequestedBounds(win, bounds, opening) {
    const key = JSON.stringify(bounds);
    if (opening || requested.get(win) !== key) {
      requested.set(win, key);
      win.setBounds(bounds);
    }
  }
  // Rust owns Linux placement. Chromium can report stale bounds after an XWayland
  // override-redirect remap; send the requested rect rather than undoing the move.
  function placeSurface(win, focus, force = false, bounds = win.getBounds()) {
    if (process.platform !== 'linux') return;
    const { x, y, width, height } = bounds;
    const rect = [x, y, width, height];
    const key = JSON.stringify(rect);
    if (!force && !focus && placed.get(win) === key) return;
    placed.set(win, key);
    send({ action: 'surface', id: win.getNativeWindowHandle().readUInt32LE(0), rect, focus });
  }
  function validate(event) {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (![popup, badge].includes(win) || event.senderFrame !== event.sender.mainFrame ||
      !event.senderFrame.url.startsWith(page + '#notes-')) throw new Error('Forbidden notes request');
  }
  function publish() {
    if (!latest) return;
    for (const win of [popup, badge]) win.webContents.send('notes:snapshot', latest);
  }
  ipcMain.handle('notes:ready', (event) => { validate(event); publish(); });
  ipcMain.handle('notes:finish-ready', event => { validate(event); if (event.sender !== popup.webContents || !finishing || finished) return; finished = true; send({ action: 'finished' }); });
  ipcMain.handle('notes:command', (event, value) => {
    validate(event);
    if (finished || !validCommand(value)) throw new Error('Invalid notes command');
    send(value);
  });
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  lines.on('line', line => {
    if (Buffer.byteLength(line) > 10_485_760) return app.quit();
    try {
      const state = JSON.parse(line);
      if (state.finishing === true && Object.keys(state).length === 1) {
        finishing = true; popup.hide(); badge.hide();
        const notify = () => popup.webContents.send('notes:finish');
        if (popup.webContents.isLoadingMainFrame()) popup.webContents.once('did-finish-load', notify); else notify();
        return;
      }
      if (finishing) return;
      if (!validSnapshot(state)) return app.quit();
      const previous = latest; latest = state;
      let requestedBadge;
      const openingBadge = !badge.isVisible();
      if (state.badgeRect) {
        const rect = state.badgeRect;
        const work = screen.getDisplayNearestPoint({ x: Math.round(rect[0] + rect[2] / 2), y: Math.round(rect[1] + rect[3] / 2) }).workArea;
        const bounds = badgeBounds(rect, work);
        requestedBadge = { ...bounds, width: 150, height: 44 };
        setRequestedBounds(badge, requestedBadge, openingBadge);
      }
      const point = { x: Math.round(state.position[0]), y: Math.round(state.position[1]) };
      if (state.mode !== 'badge') {
        const work = screen.getDisplayNearestPoint(point).workArea;
        const bounds = popupBounds(point, work, state.mode === 'list' ? 350 : 220);
        const opening = !popup.isVisible();
        setRequestedBounds(popup, bounds, opening);
        if (opening) popup.show();
        const focus = !previous || previous.mode !== state.mode || previous.current !== state.current;
        if (focus) popup.focus();
        placeSurface(popup, focus, opening, bounds);
      } else { popup.hide(); if (previous && previous.mode !== 'badge') send({ action: 'hidden' }); }
      if (openingBadge) badge.showInactive();
      placeSurface(badge, false, openingBadge, requestedBadge); publish();
    } catch { app.quit(); }
  });
  lines.on('close', () => app.quit());
  Promise.all([popup.webContents.executeJavaScript('document.readyState'), badge.webContents.executeJavaScript('document.readyState')]).then(() => {
    const ids = [popup, badge].map(w => Number(w.getMediaSourceId().split(':')[1])).filter(n => Number.isInteger(n) && n > 0);
    send({ action: 'windows', ids }); send({ action: 'ready' });
  }).catch(() => app.quit());
});
app.on('window-all-closed', () => app.quit());

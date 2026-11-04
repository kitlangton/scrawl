require('../runtime.cjs').installBuildIdentity(require('electron'));
require('../runtime.cjs').installExceptionReporting(require('electron'));
// Only Rust's private inherited stdin supplies capture state. Never opens a socket or backend.
const { app, BrowserWindow, ipcMain, screen, session, nativeTheme } = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const suppliedProfile = process.argv.find(v => v.startsWith('--notes-profile='))?.slice('--notes-profile='.length);
const profile = suppliedProfile || fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'apparent-note-ui-'));
const { pathToFileURL } = require('node:url');
const readline = require('node:readline');
// Fixed lifecycle labels only: never log note text, page contents or error messages.
const fixtureDiagnostics = process.argv.includes('--notes-fixture-diagnostics');
const trace = label => { if (fixtureDiagnostics) process.stderr.write(`Notes fixture: ${label}\n`); };
trace('entry');
const { validCommand, validSnapshot, popupBounds, badgeBounds, embeddedBadgeBounds, BADGE_WIDTH, BADGE_HEIGHT, flightBounds } = require('./protocol.cjs');
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
  trace('app-ready');
  trace('activation-policy-start');
  if (process.platform === 'darwin') app.setActivationPolicy('accessory');
  trace('activation-policy-complete');
  session.defaultSession.setPermissionRequestHandler((_wc, _permission, reply) => reply(false));
  session.defaultSession.setPermissionCheckHandler(() => false);
  const page = pathToFileURL(path.join(__dirname, '../../dist/index.html')).href;
  function window(surface, width, height) {
    trace(`${surface}:construct-start`);
    const win = new BrowserWindow({ title: 'Apparent notes', width, height, show: false, frame: false, transparent: true,
      resizable: false, skipTaskbar: true, alwaysOnTop: true, hasShadow: false,
      webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true,
        nodeIntegration: false, sandbox: true, spellcheck: false, webSecurity: true } });
    trace(`${surface}:construct-complete`);
    win.webContents.on('page-title-updated', event => event.preventDefault());
    win.removeMenu(); win.setAlwaysOnTop(true, 'floating');
    win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
    win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    win.webContents.on('will-navigate', e => e.preventDefault());
    win.webContents.on('will-attach-webview', e => e.preventDefault());
    win.webContents.on('render-process-gone', () => { trace(`${surface}:renderer-gone`); app.quit(); });
    if (fixtureDiagnostics) {
      win.webContents.on('did-finish-load', () => trace(`${surface}:loaded`));
      win.webContents.on('did-fail-load', () => trace(`${surface}:load-failed`));
      win.webContents.on('preload-error', () => trace(`${surface}:preload-failed`));
    }
    win.loadURL(page + '#notes-' + surface);
    trace(`${surface}:created`);
    return win;
  }
  popup = window('popup', 332, 200); badge = window('badge', BADGE_WIDTH, BADGE_HEIGHT);
  let flight = null, flightTimer, pendingSave = null, flightSerial = 0;
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
    for (const win of [popup, badge]) win.webContents.send('notes:snapshot', { ...latest, flight });
  }
  function cancelFlight() {
    if (!flight) return;
    clearTimeout(flightTimer); flight = null;
    popup.hide(); popup.setIgnoreMouseEvents(false);
    if (process.platform === 'darwin') popup.setFocusable(true);
    send({ action: 'flight', active: false });
  }
  function flyToCounter(target) {
    const motion = flightBounds(JSON.parse(requested.get(popup) || JSON.stringify(popup.getBounds())), target);
    if (!motion) return false;
    flight = { id: ++flightSerial, start: motion.start, delta: motion.delta };
    // The native input filter must ignore the enlarged visual-only surface too.
    send({ action: 'flight', active: true });
    popup.hide(); popup.setIgnoreMouseEvents(true);
    if (process.platform === 'darwin') popup.setFocusable(false);
    send({ action: 'hidden' });
    setRequestedBounds(popup, motion.bounds, true);
    popup.showInactive(); placeSurface(popup, false, true, motion.bounds);
    flightTimer = setTimeout(() => { cancelFlight(); publish(); }, 260);
    return true;
  }
  ipcMain.handle('notes:ready', (event) => { validate(event); publish(); });
  ipcMain.handle('notes:finish-ready', event => { validate(event); if (event.sender !== popup.webContents || !finishing || finished) return; finished = true; send({ action: 'finished' }); });
  ipcMain.handle('notes:command', (event, value) => {
    validate(event);
    if (finished || !validCommand(value)) throw new Error('Invalid notes command');
    if (value.action === 'save') pendingSave = { pin: value.pin, text: value.text.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, '') };
    send(value);
  });
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  lines.on('line', line => {
    if (Buffer.byteLength(line) > 10_485_760) return app.quit();
    try {
      const state = JSON.parse(line);
      if (state.finishing === true && Object.keys(state).length === 1) {
        finishing = true; pendingSave = null; cancelFlight(); popup.hide(); badge.hide();
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
        const centre = { x: Math.round(rect[0] + rect[2] / 2), y: Math.round(rect[1] + rect[3] / 2) };
        const display = screen.getDisplayNearestPoint(process.platform === 'linux' && state.embedded ? screen.screenToDipPoint(centre) : centre);
        const work = display.workArea;
        const bounds = state.embedded
          ? embeddedBadgeBounds(rect, process.platform === 'linux' ? display.scaleFactor : 1)
          : badgeBounds(rect, work);
        requestedBadge = state.embedded ? bounds : { ...bounds, width: BADGE_WIDTH, height: BADGE_HEIGHT };
        setRequestedBounds(badge, requestedBadge, openingBadge);
      }
      const point = { x: Math.round(state.position[0]), y: Math.round(state.position[1]) };
      const attached = pendingSave && state.mode === 'badge' && !state.error && pendingSave.text.length > 0 &&
        state.notes.some(n => n.pin === pendingSave.pin && n.text === pendingSave.text);
      if (state.mode === 'badge' || state.error || (pendingSave && state.current !== pendingSave.pin)) pendingSave = null;
      if (state.mode !== 'badge') {
        cancelFlight();
        const work = screen.getDisplayNearestPoint(point).workArea;
        const bounds = popupBounds(point, work, state.mode === 'list' ? 350 : 220);
        const opening = !popup.isVisible();
        setRequestedBounds(popup, bounds, opening);
        if (opening) popup.show();
        const focus = !previous || previous.mode !== state.mode || previous.current !== state.current;
        if (focus) {
          // A key window in an accessory app need not be the keyboard's active
          // application. Activate only for this explicit editor/list opening.
          if (process.platform === 'darwin') app.focus({ steal: true });
          popup.focus(); popup.webContents.focus();
        }
        placeSurface(popup, focus, opening, bounds);
      } else if (attached && requestedBadge) {
        cancelFlight();
        if (!flyToCounter(requestedBadge)) { popup.hide(); send({ action: 'hidden' }); }
      } else if (!flight) { popup.hide(); if (previous && previous.mode !== 'badge') send({ action: 'hidden' }); }
      if (requestedBadge && !state.embedded) {
        if (openingBadge) badge.showInactive();
        placeSurface(badge, false, openingBadge, requestedBadge);
      } else { badge.hide(); }
      publish();
    } catch { app.quit(); }
  });
  lines.on('close', () => app.quit());
  trace('ready-probe-start');
  const probe = (win, surface) => {
    const result = win.webContents.executeJavaScript('document.readyState');
    if (!fixtureDiagnostics) return result;
    return result.then(value => { trace(`${surface}:probe-complete`); return value; });
  };
  Promise.all([probe(popup, 'popup'), probe(badge, 'badge')]).then(() => {
    trace('ready-probe-complete');
    const ids = [popup, badge].map(w => Number(w.getMediaSourceId().split(':')[1])).filter(n => Number.isInteger(n) && n > 0);
    send({ action: 'windows', ids }); send({ action: 'ready' });
    trace('ready-sent');
  }).catch(() => { trace('ready-probe-failed'); app.quit(); });
});
app.on('window-all-closed', () => app.quit());

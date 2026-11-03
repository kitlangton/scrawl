const { test } = require('node:test');
const assert = require('node:assert/strict');
const { _electron: electron, expect } = require('@playwright/test');
const path = require('node:path');
const fs = require('node:fs/promises');
const os = require('node:os');

test('shipped Electron note UI focuses, saves, counts, edits, recovers drafts and isolates bridge', { timeout: 45000 }, async t => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'apparent-notes-ui-'));
  const profile = path.join(home, 'owned-profile');
  await fs.mkdir(profile, { mode: 0o700 });
  const application = await electron.launch({ args:
    ['--no-sandbox', path.resolve(__dirname, '../pill/host.cjs'), '--notes-fixture', '--capture-frame=0,0,1280,800', '--notes-profile=' + profile],
    env: { ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, 'config') } });
  const child = application.process();
  let errors = '';
  child.stderr.on('data', chunk => { errors += chunk.toString(); });
  t.after(async () => { await application.close().catch(()=>{}); child.kill(); await new Promise(resolve => child.exitCode !== null ? resolve() : child.once('exit', resolve)); await fs.rm(home, { recursive: true, force: true }); });
  const messages = [];
  let pending = '';
  child.stdout.on('data', chunk => {
    pending += chunk.toString(); const lines = pending.split('\n'); pending = lines.pop();
    for (const line of lines) { try {
      const value = JSON.parse(line); messages.push(value);
      // Production Rust publishes each accepted draft back to the renderer.
      if (value.action === 'draft' && state?.mode === 'editor' && state.current === value.pin) {
        state = { ...state, drafts: state.drafts.map(n => n.pin === value.pin ? { ...n, text: value.text } : n) }; send();
      }
    } catch {} }
  });
  const context = application.context();
  await expect.poll(() => context.pages().length).toBe(2);
  const pages = context.pages();
  const popup = pages.find(p => p.url().endsWith('#notes-popup')) || pages[0];
  const badge = pages.find(p => p !== popup);
  await popup.waitForFunction(() => !!window.captureNotes);
  let state = { mode: 'editor', current: 1, position: [600, 280], badgeRect: [450, 36, 380, 40], notes: [], drafts: [{ pin: 1, t: 123, x: .5, y: .35, text: '' }], error: null };
  const send = () => { void application.evaluate((_electron,line)=>process.stdin.emit('data',line+'\n'),JSON.stringify(state)); };
  send();
  const field = popup.getByRole('textbox', { name: 'Note text' });
  await expect(field).toBeFocused();
  const { backgroundColor } = require('@apparent/ui/window-theme');
  for (const mode of ['light', 'dark', 'light']) {
    await popup.emulateMedia({ colorScheme: mode });
    const hex = backgroundColor({ shouldUseDarkColors: mode === 'dark' });
    const rgb = hex.slice(1).match(/../g).map(value => parseInt(value, 16));
    await expect(popup.locator('.note-popup')).toHaveCSS('background-color', `rgb(${rgb.join(', ')})`);
    await expect(popup.locator('body')).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  }
  assert.deepEqual(await popup.evaluate(() => ({ require: typeof window.require, process: typeof window.process, keys: Object.keys(window.captureNotes).sort() })),
    { require: 'undefined', process: 'undefined', keys: ['command', 'onSnapshot'] });
  await popup.evaluate(() => {
    document.activeElement.blur();
    window.dispatchEvent(new Event('focus'));
  });
  await expect(field).toBeFocused();
  await field.pressSequentially('Smaller button', { delay: 20 }); await expect(field).toBeFocused(); await field.press('Shift+Enter'); await field.press('x');
  await expect(field).toHaveValue('Smaller button\nx');
  await field.press('Enter');
  await expect.poll(() => messages.find(m => m.action === 'save')?.text).toBe('Smaller button\nx');
  await popup.emulateMedia({ reducedMotion: 'reduce' });
  await popup.evaluate(() => {
    window.flightSeen = false;
    new MutationObserver(() => { const sprite = document.querySelector('.note-flight'); if (sprite) { window.flightSeen = true; window.flightDisplay = getComputedStyle(sprite).display; window.flightBody = getComputedStyle(document.body).backgroundColor; } })
      .observe(document.body, { subtree: true, childList: true });
  });
  // Real Rust owns this ack in production. This fixture tests renderer behavior only.
  state = { ...state, mode: 'badge', notes: [{ ...state.drafts[0], text: 'Smaller button\nx' }], drafts: [{ ...state.drafts[0], text: 'Smaller button\nx' }] }; send();
  await expect(badge.getByRole('button', { name: '1 saved notes. Open notes' })).toBeVisible();
  await expect.poll(() => popup.evaluate(() => window.flightSeen)).toBe(true);
  assert.equal(await popup.evaluate(() => window.flightDisplay), 'none', 'reduced motion hides the decorative flight');
  assert.equal(await popup.evaluate(() => window.flightBody), 'rgba(0, 0, 0, 0)');
  await expect.poll(() => messages.some(m => m.action === 'flight' && m.active)).toBe(true);
  await expect.poll(() => messages.some(m => m.action === 'flight' && !m.active)).toBe(true);

  await badge.getByRole('button').click();
  await expect.poll(() => messages.some(m => m.action === 'list')).toBe(true);
  state.mode = 'list'; state.badgeRect = [450, 36, 380, 40]; send();
  if (process.platform === 'linux') {
    await expect.poll(() => messages.some(m => m.action === 'surface' && JSON.stringify(m.rect) === '[846,34,116,44]')).toBe(true);
    await expect.poll(() => messages.some(m => m.action === 'surface' && m.rect[3] === 350)).toBe(true);
  }
  await popup.getByRole('button', { name: /Smaller button/ }).click();
  await expect.poll(() => messages.some(m => m.action === 'edit' && m.pin === 1)).toBe(true);
  state.mode = 'editor'; send();
  await expect(field).toBeFocused(); await field.fill('Draft correction'); await field.press('Escape');
  await expect.poll(() => messages.some(m => m.action === 'dismiss')).toBe(true);
  state.mode = 'badge'; state.drafts[0].text = 'Draft correction'; send();
  state.mode = 'editor'; send(); await expect(field).toHaveValue('Draft correction');
  const dismissals = messages.filter(m => m.action === 'dismiss').length;
  await popup.locator('main').click({ position: { x: 10, y: 210 } });
  await expect.poll(() => messages.filter(m => m.action === 'dismiss').length).toBe(dismissals + 1);
  await field.press('Enter');
  state.error = 'Could not save this note. Your text is kept; try again.'; send();
  await expect(popup.getByRole('alert')).toBeVisible(); await expect(field).toHaveValue('Draft correction');
  await assert.rejects(popup.evaluate(() => window.captureNotes.command({ action: 'save', pin: 1, text: 'x', path: '/tmp/no' })));
  // Only the pill counter is replaced; the existing editor/list remains available.
  state = { ...state, embedded: true }; send();
  const badgeVisible = () => application.evaluate(({BrowserWindow}) => BrowserWindow.getAllWindows().find(w=>w.webContents.getURL().endsWith('#notes-badge')).isVisible());
  await expect.poll(badgeVisible).toBe(false);
  await expect(field).toHaveValue('Draft correction');
  state = { ...state, embedded: false }; send();
  await expect.poll(badgeVisible).toBe(true);
  // Finish must acknowledge every save already sent by this renderer, in pipe order.
  await popup.evaluate(() => { void window.captureNotes.command({ action: 'save', pin: 1, text: 'Final correction\n' }); });
  await popup.emulateMedia({ reducedMotion: 'no-preference' });
  const flightsBefore = messages.filter(m => m.action === 'flight' && m.active).length;
  state = { ...state, mode: 'badge', error: null,
    notes: [{ ...state.drafts[0], text: 'Final correction' }], drafts: [{ ...state.drafts[0], text: 'Final correction' }] }; send();
  await expect.poll(() => messages.filter(m => m.action === 'flight' && m.active).length).toBe(flightsBefore + 1);
  await application.evaluate(()=>process.stdin.emit('data',JSON.stringify({ finishing: true })+'\n'));
  await expect.poll(() => messages.some(m => m.action === 'finished')).toBe(true);
  const finish = messages.findIndex(m => m.action === 'finished');
  const lastFlight = messages.findLastIndex(m => m.action === 'flight' && m.active);
  assert.ok(messages.slice(lastFlight + 1, finish).some(m => m.action === 'flight' && !m.active),
    'Finish cancels an active decorative animation before its save barrier');
  assert.ok(messages.findIndex(m => m.action === 'save' && m.text === 'Final correction\n') < finish);
  await assert.rejects(popup.evaluate(() => window.captureNotes.command({ action: 'save', pin: 1, text: 'Too late' })));
  const exited = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Notes did not exit after EOF')), 5000);
    child.once('exit', code => { clearTimeout(timer); resolve(code); });
  });
  await application.evaluate(()=>process.stdin.push(null));
  assert.equal(await exited, 0);
  assert.equal((await fs.stat(profile)).isDirectory(), true, 'the parent owns profile cleanup after reaping');
  assert.doesNotMatch(errors, /Uncaught Exception|ENOTEMPTY/);
});

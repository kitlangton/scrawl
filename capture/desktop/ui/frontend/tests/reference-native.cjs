// Real Electron -> owned Rust GUI -> system clipboard. Generated fixtures only.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const { spawn, execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { _electron: electron, expect } = require('@playwright/test');
const { captureReference } = require('../electron/runtime.cjs');
const run = promisify(execFile);

test('selected capture reference: real Rust selection and native clipboard', { timeout: 120000 }, async (t) => {
  const binary = process.env.APPARENT_REFERENCE_BINARY;
  const output = process.env.APPARENT_REFERENCE_OUTPUT;
  assert.ok(binary && path.isAbsolute(binary) && output && path.isAbsolute(output), 'Explicit binary and fresh output required');
  let proof;
  if (process.platform === 'darwin') {
    assert.equal(process.env.GITHUB_ACTIONS, 'true');
    assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
    assert.ok(binary.includes('.app/Contents/MacOS/apparent'), 'Mac uses the installed App');
    await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', path.resolve(binary, '../../..')]);
    proof = 'macOS GitHub-hosted native Electron/Rust clipboard';
  } else {
    assert.equal(process.platform, 'linux');
    if (process.env.APPARENT_REFERENCE_VM === '1') {
      await run('systemd-detect-virt', ['--vm', '--quiet']);
      proof = 'isolated Linux VM native Electron/Rust clipboard';
    } else {
      assert.equal(process.env.APPARENT_REFERENCE_XVFB, '1', 'Refuse a personal desktop');
      const number = /^:(\d+)/.exec(process.env.DISPLAY ?? '')?.[1];
      assert.ok(number);
      const pid = (await fs.readFile(`/tmp/.X${number}-lock`, 'utf8')).trim();
      assert.match(pid, /^\d+$/);
      assert.equal((await fs.readFile(`/proc/${pid}/comm`, 'utf8')).trim(), 'Xvfb');
      proof = 'Xvfb integration only; not Linux native acceptance';
    }
  }
  await fs.mkdir(output, { mode: 0o700 });
  const home = await fs.mkdtemp(path.join(os.homedir(), '.reference-'));
  await fs.chmod(home, 0o700);
  const config = path.join(home, 'config.json');
  const captures = path.join(home, 'captures');
  await fs.mkdir(captures);
  assert.ok(Buffer.byteLength(path.join(home, '.config/apparent/agent-control/capture.sock')) < 104);
  const env = { ...process.env, HOME: home, CFFIXED_USER_HOME: home,
    XDG_CONFIG_HOME: path.join(home, '.config'), APPARENT_CONFIG: config,
    APPARENT_CAPTURES_DIR: captures, APPARENT_NO_ONBOARDING: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  if (process.platform === 'linux') env.LD_LIBRARY_PATH = `${path.dirname(binary)}:${env.LD_LIBRARY_PATH ?? ''}`;
  const report = { passed: false, proof, binarySHA256: crypto.createHash('sha256').update(await fs.readFile(binary)).digest('hex'), checks: [] };
  let rust;
  let application;
  let log;
  let seedOwner;
  async function stopOwned(child) {
    if (child && child.exitCode === null && child.signalCode === null) {
      const ended = new Promise((resolve) => child.once('exit', resolve));
      child.kill('SIGTERM');
      const timer = setTimeout(() => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); }, 2500);
      try { await ended; } finally { clearTimeout(timer); }
    }
  }
  async function stopRust() {
    await stopOwned(rust);
    await log?.close(); log = undefined;
    rust = undefined;
  }
  t.after(async () => {
    try {
      if (!report.passed && application) {
        try { await (await application.firstWindow()).screenshot({ path: path.join(output, 'failure.png'), fullPage: true }); }
        catch { report.failureScreenshotUnavailable = true; }
      }
      const stopped = await Promise.allSettled([application?.close(), stopRust(), stopOwned(seedOwner)]);
      const failures = stopped.filter((result) => result.status === 'rejected');
      if (failures.length) throw new AggregateError(failures.map((result) => result.reason), 'Owned cleanup failed');
      await fs.rm(home, { recursive: true, force: true });
      report.ownedCleanupVerified = true;
    } catch (error) { report.passed = false; report.cleanupError = error.message; throw error; }
    finally { await fs.writeFile(path.join(output, 'reference.json'), JSON.stringify(report, null, 2)); }
  });
  await fs.writeFile(config, JSON.stringify({ onboarding: { seen: true } }), { mode: 0o600 });
  const newer = 'newer selected.apparent';
  const older = 'older capture — selected.apparent';
  for (const name of [newer, older]) {
    await run(binary, ['demo', '--out', path.join(captures, name)], { env, timeout: 20000, maxBuffer: 1048576 });
  }
  async function startRust(name) {
    await stopRust();
    log = await fs.open(path.join(output, `rust-${report.checks.length}.log`), 'a', 0o600);
    rust = spawn(binary, name ? ['preview', path.join(captures, name)] : ['app'], { env, stdio: ['ignore', log.fd, log.fd] });
    rust.once('error', (error) => { report.launchError = error.message; });
    await expect.poll(async () => {
      assert.equal(rust.exitCode, null, 'Owned backend exited');
      const r = await captureReference(home);
      return r.status === 'connected' ? r.id : r.status;
    }, { timeout: 15000 }).toBe(name ?? null);
  }
  await startRust(newer);
  application = await electron.launch({ args: [path.join(__dirname, '..')], env });
  const page = await application.firstWindow();
  await expect.poll(() => application.evaluate(({ BrowserWindow }) => {
    const windows = BrowserWindow.getAllWindows();
    if (windows.length !== 1) return false;
    windows[0].focus();
    return windows[0].isFocused();
  })).toBe(true);
  const card = page.locator('section[aria-labelledby="reference-title"]');
  const copy = card.getByRole('button', { name: 'Copy capture ID', exact: true });
  const refresh = card.getByRole('button', { name: 'Refresh capture reference' });
  await card.getByText(newer, { exact: true }).waitFor();
  const image = path.join(captures, newer, 'contact-sheet.png');
  async function seed() {
    const png = (await fs.readFile(image)).toString('base64');
    if (process.env.APPARENT_REFERENCE_VM === '1') {
      // Publish into the compositor rather than trusting Electron's own offer.
      await stopOwned(seedOwner);
      seedOwner = spawn('wl-copy', ['--foreground', '--type', 'image/png'], { env, stdio: ['pipe', 'ignore', 'inherit'] });
      seedOwner.once('error', (error) => { report.seedError = error.message; });
      seedOwner.stdin.on('error', (error) => { report.seedError = error.message; });
      seedOwner.stdin.end(Buffer.from(png, 'base64'));
      const imageSHA256 = crypto.createHash('sha256').update(Buffer.from(png, 'base64')).digest('hex');
      let seeded;
      await expect.poll(async () => { seeded = await readClipboard(); return seeded.imageSHA256; }).toBe(imageSHA256);
      report.nativeClipboardBoundary = 'Wayland PNG seed hash verified through wl-paste';
      return seeded;
    }
    await application.evaluate(async ({ clipboard, ClipboardItem }, png) => {
      await clipboard.write([new ClipboardItem({ 'text/plain': 'clipboard sentinel',
        'image/png': new Blob([Buffer.from(png, 'base64')], { type: 'image/png' }) })]);
    }, png);
    const seeded = await readClipboard();
    assert.equal(seeded.text, 'clipboard sentinel');
    assert.ok(seeded.formats.includes('image/png'), 'PNG seed was actually published');
    return seeded;
  }
  async function readClipboard() {
    if (process.env.APPARENT_REFERENCE_VM === '1') {
      // Read the compositor's actual selection, not an Electron/XWayland
      // owner's cached offer. The PNG seed must reach this same boundary.
      const unstable = { text: null, formats: [], imageSHA256: null, unstable: true };
      const offered = async () => [...new Set((await run('wl-paste', ['--list-types'], { timeout: 3000 })).stdout.trim().split('\n').filter(Boolean))].sort();
      try {
        const types = await offered();
        const text = types.some((type) => type.startsWith('text/'))
          ? (await run('wl-paste', ['--no-newline', '--type', 'text'], { timeout: 3000 })).stdout : null;
        const imageSHA256 = types.includes('image/png') ? crypto.createHash('sha256').update(
          (await run('wl-paste', ['--type', 'image/png'], { timeout: 3000, encoding: null, maxBuffer: 16 * 1024 * 1024 })).stdout).digest('hex') : null;
        if (JSON.stringify(types) !== JSON.stringify(await offered())) return unstable;
        return { text, formats: types, imageSHA256 };
      } catch (error) {
        // Selection ownership can change between these read-only commands.
        // Return an explicitly non-matching snapshot; never retry the copy.
        if (/^(Nothing is copied|Clipboard content is not available as requested type)/.test(error.stderr ?? '')) return unstable;
        throw error;
      }
    }
    return application.evaluate(async ({ clipboard }) => ({ text: await clipboard.readText(),
      formats: (await clipboard.read()).flatMap((item) => item.types) }));
  }
  async function copied(name, label) {
    await seed(); await copy.click();
    let result;
    await expect.poll(async () => { result = await readClipboard(); return result.text; }).toBe(name);
    report.checks.push({ label, ...result });
    // Electron 45 exposes raw OS types as well as MIME types. TARGETS and
    // SAVE_TARGETS are X11 selection protocol metadata, not clipboard payloads.
    const textFormats = new Set(['text/plain', 'text/plain;charset=utf-8', 'text/plain;charset=UTF-8',
      'UTF8_STRING', 'STRING', 'TEXT', 'COMPOUND_TEXT', 'public.utf8-plain-text',
      'public.utf16-plain-text', 'public.utf16-external-plain-text', 'NSStringPboardType',
      'TARGETS', 'SAVE_TARGETS']);
    assert.ok(result.formats.length > 0 && result.formats.every((s) =>
      textFormats.has(/^electron application\/osclipboard;format="([^"]+)"$/.exec(s)?.[1] ?? s)), JSON.stringify(result));
    await card.getByText('Capture ID copied', { exact: true }).waitFor();
    await page.screenshot({ path: path.join(output, `${label}.png`), fullPage: true });
  }
  await copied(newer, 'newer');
  await copied(newer, 'repeated-fresh-dispatch');
  await startRust(older); // Actual production preview selection, not a fake reply.
  const staleSeed = await seed(); await copy.click();
  await card.getByText('The selected capture changed. Refresh before copying again.', { exact: true }).waitFor();
  assert.deepEqual(await readClipboard(), staleSeed);
  report.checks.push({ label: 'stale selection refused without clipboard write' });
  await refresh.click(); await card.getByText(older, { exact: true }).waitFor();
  await copied(older, 'older');
  await application.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(640, 760));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await copied(older, 'minimum-width');
  assert.equal(await application.evaluate(async ({ ipcMain, BrowserWindow }) => {
    const sender = BrowserWindow.getAllWindows()[0].webContents;
    for (const operation of ['read', 'copy']) {
      const handler = ipcMain._invokeHandlers.get(`apparent:${operation}-capture-reference`);
      for (const [frame, args] of [[sender.mainFrame, ['arbitrary.apparent']], [{ url: sender.getURL() }, []]]) {
        try { await handler({ sender, senderFrame: frame }, ...args); return false; } catch {}
      }
    }
    return true;
  }), true);
  await startRust(null); await refresh.click();
  await card.getByText('No saved capture is selected. Select one in the current app, then refresh.', { exact: true }).waitFor();
  await expect(copy).toBeDisabled();
  await page.screenshot({ path: path.join(output, 'no-selection.png'), fullPage: true });
  await stopRust();
  await fs.writeFile(config, JSON.stringify({ onboarding: { seen: true }, experiments: { copyCaptureId: false } }));
  log = await fs.open(path.join(output, 'rust-disabled.log'), 'a', 0o600);
  rust = spawn(binary, ['preview', path.join(captures, older)], { env, stdio: ['ignore', log.fd, log.fd] });
  await expect.poll(async () => (await captureReference(home)).status, { timeout: 15000 }).toBe('disabled');
  await refresh.click(); await expect(card).toHaveCount(0);
  await page.screenshot({ path: path.join(output, 'explicit-false.png'), fullPage: true });
  report.checks.push({ label: 'no selection disables copy; explicit false hides action; argument/frame guards reject' });
  report.passed = true;
});

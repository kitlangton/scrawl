// A bounded installed-package check, separate from capture/permission acceptance.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { spawn, execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { createHash } = require('node:crypto');
const run = promisify(execFile);
const { within, stopElectron, supervisor, stopBackend } = require('../../../../enablement/native/owned-processes.ts');
const frontend = path.resolve(__dirname, '../../ui/frontend');

// A location render starts retention/usage reads on Rust's same bounded worker.
// Retry only explicit worker contention, never malformed replies or other errors.
async function readLocation(read, budget, replies) {
  for (let attempt = 0; attempt < 8; attempt++) {
    const result = await within(read(), budget());
    replies.push(result);
    if (result.status !== 'error' || result.code !== 'app_busy') return result;
    await within(new Promise(resolve => setTimeout(resolve, 150)), budget());
  }
  throw new Error('Storage worker remained busy after eight bounded reads');
}

function admission(env, platform) {
  assert.equal(platform, 'darwin', 'Packaged Mac smoke requires macOS');
  for (const [key, value] of Object.entries({ CI: 'true', GITHUB_ACTIONS: 'true',
    RUNNER_ENVIRONMENT: 'github-hosted', APPARENT_PACKAGED_SMOKE: '1' })) {
    assert.equal(env[key], value, `Disposable hosted runner required: ${key}`);
  }
  assert.ok(env.APPARENT_APP_DIR && path.isAbsolute(env.APPARENT_APP_DIR));
  assert.ok(env.APPARENT_SMOKE_OUTPUT && path.isAbsolute(env.APPARENT_SMOKE_OUTPUT));
}

async function main() {
  admission(process.env, process.platform);
  const deadline = Date.now() + 75000;
  const budget = (cap = 5000) => {
    const remaining = deadline - Date.now();
    assert.ok(remaining > 0, 'Packaged smoke deadline exceeded');
    return Math.min(cap, remaining);
  };
  const { _electron: electron, expect } = require(path.join(frontend, 'node_modules/@playwright/test'));
  const { readRuntimeState } = require(path.join(frontend, 'electron/runtime.cjs'));
  const appDir = await fs.realpath(process.env.APPARENT_APP_DIR);
  const packaged = path.join(appDir, 'Contents/Resources/settings');
  const entry = path.join(packaged, 'app');
  const runtime = path.join(packaged, 'runtime/Electron.app/Contents/MacOS/Electron');
  const binary = path.join(appDir, 'Contents/MacOS/apparent');
  // Fail on missing installed inputs; never fall back to the checkout's runtime.
  for (const file of [runtime, binary, path.join(entry, 'electron/settings.cjs'),
    path.join(entry, 'dist/settings.html')]) await fs.access(file);
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', appDir], { timeout: 15000 });
  const output = process.env.APPARENT_SMOKE_OUTPUT;
  await fs.mkdir(output, { mode: 0o700 }); // Exclusive output; preserve failed evidence.
  const report = { passed: false, platform: process.platform, arch: process.arch,
    scope: 'Installed Storage renderer, real Rust readback, close and reopen only',
    nativeCaptureAcceptance: 'UNRUN', packagedApp: appDir, cycles: [], cleanup: {} };
  const home = await fs.mkdtemp(path.join(await fs.realpath(process.env.RUNNER_TEMP), 'ps-'));
  await fs.chmod(home, 0o700);
  const config = path.join(home, '.config/apparent/config.json');
  const captures = path.join(home, 'synthetic-captures');
  const env = { ...process.env, HOME: home, CFFIXED_USER_HOME: home,
    APPARENT_CONFIG: config, XDG_CONFIG_HOME: path.join(home, '.config'),
    TMPDIR: home, TMP: home, TEMP: home };
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.APPARENT_CAPTURES_DIR;
  let backend, application, page, log, failure;
  try {
    await fs.mkdir(path.dirname(config), { recursive: true, mode: 0o700 });
    await fs.mkdir(captures, { mode: 0o700 });
    assert.ok(Buffer.byteLength(path.join(home, '.config/apparent/agent-control/capture.sock')) <= 103);
    await fs.writeFile(config, JSON.stringify({ onboarding: { seen: true },
      capturesDir: captures, errorReporting: false }), { mode: 0o600, flag: 'wx' });
    report.binarySha256 = createHash('sha256').update(await fs.readFile(binary)).digest('hex');
    report.commit = (await run('git', ['rev-parse', 'HEAD'], { cwd: frontend, timeout: 15000 })).stdout.trim();
    log = await fs.open(path.join(output, 'backend.log'), 'wx', 0o600);
    backend = spawn('/bin/bash', ['-c', supervisor, 'packaged-smoke', binary, 'app'],
      { env, detached: true, stdio: ['pipe', log.fd, log.fd] });
    let spawnError;
    backend.on('error', error => { spawnError = error; });
    const until = Date.now() + budget(20000);
    let ready = false;
    while (Date.now() < until) {
      if (spawnError) throw spawnError;
      assert.equal(backend.exitCode, null, 'Backend supervisor exited');
      const state = await readRuntimeState(home, 2000);
      if (state.status === 'connected' && state.phase === 'idle') { ready = true; break; }
      await new Promise(resolve => setTimeout(resolve, 150));
    }
    assert.ok(ready, 'Installed Rust control bridge did not become idle');
    for (let cycle = 0; cycle < 2; cycle++) {
      application = await electron.launch({ executablePath: runtime, args: [entry], env, timeout: budget(15000) });
      page = await application.firstWindow({ timeout: budget(10000) });
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await expect(page.getByRole('heading', { name: 'Storage', exact: true })).toBeVisible({ timeout: budget() });
      await expect(page.getByRole('region', { name: 'Capture location', exact: true })
        .getByText('~/synthetic-captures', { exact: true })).toBeVisible({ timeout: budget() });
      await page.screenshot({ path: path.join(output, `settings-${cycle}.png`), timeout: budget() });
      const replies = [];
      report.cycles.push({ rendered: true, replies });
      const location = await readLocation(() => page.evaluate(() => window.apparent.readCaptureLocation()), budget, replies);
      assert.equal(location.status, 'connected', JSON.stringify(location));
      assert.equal(location.storage.effectivePath, captures);
      const identity = await within(application.evaluate(({ app, BrowserWindow }) => ({
        executable: process.execPath, appPath: app.getAppPath(), profile: app.getPath('userData'),
        windows: BrowserWindow.getAllWindows().length,
      })), budget());
      assert.equal(await fs.realpath(identity.executable), await fs.realpath(runtime));
      assert.equal(await fs.realpath(identity.appPath), await fs.realpath(entry));
      assert.ok(identity.profile.startsWith(home + path.sep));
      assert.equal(identity.windows, 1);
      await page.screenshot({ path: path.join(output, `settings-${cycle}.png`), timeout: budget() });
      assert.deepEqual(errors, []);
      await within(application.close(), budget(3000));
      application = undefined;
      Object.assign(report.cycles[cycle], { realRustReadback: true, closed: true, identity });
    }
    assert.equal(JSON.parse(await fs.readFile(config, 'utf8')).capturesDir, captures);
    report.passed = true;
  } catch (error) {
    failure = error; report.error = error.stack;
    if (page) {
      try { await page.screenshot({ path: path.join(output, 'failure.png'), timeout: 3000 }); }
      catch (shotError) { report.failureScreenshotUnavailable = shotError.message; }
    }
  }
  finally {
    try { await stopElectron(application); }
    catch (error) { failure ??= error; report.cleanup.electronError = error.message; }
    try { await stopBackend(backend); report.cleanup.backendStopped = true; }
    catch (error) { failure ??= error; report.cleanup.backendError = error.message; }
    if (log) await log.close();
    if (!failure) { await fs.rm(home, { recursive: true }); report.cleanup.fixtureRemoved = true; }
    else report.cleanup.retainedFixture = home;
    report.passed = report.passed && !failure;
    await fs.writeFile(path.join(output, 'packaged-smoke.json'), JSON.stringify(report, null, 2) + '\n',
      { mode: 0o600, flag: 'wx' });
  }
  if (failure) throw failure;
}

module.exports = { admission, stopBackend, stopElectron, supervisor, within, readLocation };
if (require.main === module) {
  main().catch(error => { console.error(error); process.exitCode = 1; });
}

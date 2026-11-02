// Explicit disposable GitHub macOS fixture, never a production backend launcher.
// Run after build + install-app.sh with APPARENT_NO_LAUNCH=1 and a fresh app dir.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { createHash } = require('node:crypto');
const { spawn, execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { _electron: electron, expect } = require('@playwright/test');
const run = promisify(execFile);
const root = path.join(__dirname, '..');
const permissionSource = 'https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/configure-tccdb-macos.sh';

// Retain the process-group leader while the installed GUI and helpers run.
// TERM interrupts read with status >128, not EOF. Keep stdin open until the
// anchored group stop; EOF also stops this owned group if the driver dies.
const rustSupervisor = `
set +e
set +m
trap ':' TERM
pid_file=$1
shift
if [ "$pid_file" != "\${TMPDIR%/}/rust.pid" ] || [ ! -f "$pid_file" ] || [ -L "$pid_file" ] || [ ! -O "$pid_file" ]; then
  exit 64
fi
"$@" &
printf '%s\\n' "$!" > "$pid_file"
while :; do
  IFS= read -r line
  status=$?
  if [ "$status" -eq 1 ]; then
    kill -TERM -- -$$ 2>/dev/null || true
    sleep 0.2
    kill -KILL -- -$$
  fi
done
`;

// Resolve only window 1 of the exact owned PID, then descend its sheets. Do not
// search other processes, use coordinates, or guess which Return commits a picker.
// Unknown AX shapes fail with bounded diagnostics from this process alone.
const panelScript = `
on run argv
  set targetPID to (item 1 of argv) as integer
  set operation to item 3 of argv
  tell application "System Events"
    set owned to first application process whose unix id is targetPID
    tell owned
      if (count of windows) is 0 then error "Owned process has no AX windows"
      set outerWindow to window 1
      set picker to outerWindow
      set depth to 0
      set trace to ""
      repeat
        set panelName to ""
        try
          if name of picker is not missing value then set panelName to name of picker as text
        end try
        set panelRole to role of picker as text
        set labels to name of every button of picker
        set trace to trace & "depth=" & depth & ";role=" & panelRole & ";title=" & panelName & ";buttons="
        repeat with labelText in labels
          set trace to trace & (labelText as text) & "|"
        end repeat
        set trace to trace & linefeed
        set childSheets to sheets of picker
        if (count of childSheets) is 0 then exit repeat
        if (count of childSheets) is not 1 or depth is 4 then error "Unsupported owned AX sheet tree: " & trace
        set picker to item 1 of childSheets
        set depth to depth + 1
      end repeat
      -- Failure diagnostics must not depend on the recursive selector succeeding.
      if operation is "diagnose" then
        set directChildren to UI elements of picker
        set trace to trace & "immediate-child-count=" & (count of directChildren) & linefeed
        set childLimit to count of directChildren
        if childLimit > 16 then set childLimit to 16
        repeat with childIndex from 1 to childLimit
          set directChild to item childIndex of directChildren
          set trace to trace & "child-role=" & (role of directChild as text) & ";next-roles="
          set nextRoles to role of every UI element of directChild
          if (count of nextRoles) > 32 then
            set trace to trace & "over-32"
          else
            repeat with nextRole in nextRoles
              set trace to trace & (nextRole as text) & "|"
            end repeat
          end if
          set trace to trace & linefeed
        end repeat
        return trace
      end if
      if depth > 0 and (name of outerWindow as text) is not "Apparent — UI foundation" then error "Picker has unexpected parent: " & trace
      if panelRole is not "AXSheet" and panelRole is not "AXWindow" then error "Unexpected picker role: " & trace
      -- A sheet can have no title. A named picker must expose the shipped title.
      if panelName is not "" and panelName is not "Choose capture location" then error "Unexpected picker title: " & trace
      -- Observed macOS 26 NSOpenPanel: one AXSplitGroup immediately under the
      -- sheet, with the footer buttons directly under that group. Do not walk
      -- the file-browser descendants (which can expose an entire volume).
      set controls to picker
      if (count of buttons of controls) is 0 then
        set containers to UI elements of picker
        if (count of containers) is not 1 then error "Unsupported picker control containers: " & trace
        set controls to item 1 of containers
        if (role of controls as text) is not "AXSplitGroup" then error "Unsupported picker control container role: " & trace
      end if
      set chooseMatches to {}
      set cancelMatches to {}
      set trace to trace & "footer-buttons="
      set footerButtons to buttons of controls
      if (count of footerButtons) > 16 then error "Oversized picker footer: " & trace
      repeat with foundButton in footerButtons
        set buttonName to name of foundButton
        if buttonName is not missing value then
          set trace to trace & (buttonName as text) & "|"
          if buttonName is "Choose" then set end of chooseMatches to contents of foundButton
          if buttonName is "Cancel" then set end of cancelMatches to contents of foundButton
        end if
      end repeat
      set trace to trace & linefeed
      if (count of chooseMatches) is not 1 or (count of cancelMatches) is not 1 then error "Owned native picker controls missing or ambiguous: " & trace
      set chooseButton to item 1 of chooseMatches
      set cancelButton to item 1 of cancelMatches
      if not (enabled of chooseButton) then error "Native Choose not enabled: " & trace
      set xy to position of outerWindow
      set wh to size of outerWindow
      if operation is "go" or operation is "choose" or operation is "cancel" then
        set frontmost to true
        if not frontmost then error "Owned Electron process is not frontmost"
        if operation is "go" then
          keystroke "g" using {command down, shift down}
          delay 0.4
          if not frontmost then error "Focus left owned Electron process"
          -- argv, not script interpolation: spaces, quotes and accents remain data.
          keystroke (item 2 of argv)
          key code 36
        else if operation is "choose" then
          click chooseButton
        else
          click cancelButton
        end if
      end if
      return trace & "bounds=" & (item 1 of xy) & "," & (item 2 of xy) & "," & (item 1 of wh) & "," & (item 2 of wh)
    end tell
  end tell
end run
`;

async function within(promise, ms, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
    })]);
  } finally { clearTimeout(timer); }
}

async function stopOwned(child, label, group = false) {
  if (!child) return { started: false };
  if (!child.pid) return { started: false, spawnFailed: true };
  const result = { pid: child.pid, group, exitCode: child.exitCode, signalCode: child.signalCode };
  const exited = () => child.exitCode !== null || child.signalCode !== null;
  const groupAbsent = () => {
    if (!group) return true;
    try { process.kill(-child.pid, 0); return false; }
    catch (error) { if (error.code === 'ESRCH') return true; throw error; }
  };
  const wait = async (ms) => {
    const until = Date.now() + ms;
    while ((!exited() || !groupAbsent()) && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 50));
    return exited() && groupAbsent();
  };
  const signal = (value) => {
    assert.ok(!exited(), `${label}: leader exited; refusing an unanchored signal`);
    try {
      if (group) process.kill(-child.pid, value); else child.kill(value);
      result[value] = true;
    } catch (error) {
      if (error.code !== 'ESRCH') throw error;
      result[value] = 'already absent';
    }
  };
  if (exited()) {
    assert.ok(groupAbsent(), `${label}: leader exited with a surviving group; refusing unanchored cleanup`);
    return { ...result, exited: true, groupAbsent: group ? true : undefined };
  }
  // Stop descendants while the supervisor is still the live group anchor.
  // Reap it last; never signal a group after its leader exits.
  signal('SIGTERM');
  if (!await wait(2000)) { signal('SIGKILL'); await wait(1500); }
  Object.assign(result, { exitCode: child.exitCode, signalCode: child.signalCode,
    exited: exited(), groupAbsent: group ? groupAbsent() : undefined });
  assert.ok(result.exited && groupAbsent(), `${label} did not exit completely; owned PID/group ${child.pid}`);
  return result;
}

test('real installed Mac Rust + shipped Electron Storage native Choose/Cancel/Reset', {
  timeout: 120000,
  skip: process.platform !== 'darwin' ? 'Native macOS CI only; Linux is not native proof' : false,
}, async () => {
  assert.equal(process.env.CI, 'true', 'Refuse a personal Mac');
  assert.equal(process.env.GITHUB_ACTIONS, 'true', 'GitHub ephemeral runner required');
  assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted', 'Refuse self-hosted or personal Macs');
  assert.equal(process.env.RUNNER_OS, 'macOS');
  assert.equal(process.env.APPARENT_STORAGE_NATIVE_CI, '1', 'Explicit native CI opt-in required');
  assert.ok(!process.versions.electron || process.env.ELECTRON_RUN_AS_NODE === '1',
    'Only the test driver may use Electron Node mode; GUI children must not');
  const runner = await fs.realpath(process.env.RUNNER_TEMP);
  const output = process.env.APPARENT_STORAGE_NATIVE_OUTPUT;
  const appDir = process.env.APPARENT_STORAGE_NATIVE_APP_DIR;
  assert.ok(output && path.isAbsolute(output), 'Private evidence output directory required');
  assert.ok(appDir && path.isAbsolute(appDir), 'Already installed owned .app required');
  const executable = path.join(appDir, 'Contents/MacOS/apparent');
  const helperExecutable = path.join(appDir, 'Contents/MacOS/apparent-helper');
  assert.ok((await fs.stat(executable)).isFile());
  await fs.access(executable, fs.constants.X_OK);
  await fs.access(path.join(root, 'dist/index.html'));
  await fs.mkdir(output, { recursive: true, mode: 0o700 });
  const outStat = await fs.lstat(output);
  assert.ok(outStat.isDirectory() && !outStat.isSymbolicLink() && outStat.uid === process.getuid());
  await fs.chmod(output, 0o700);
  const reportPath = path.join(output, 'mac-storage-native.json');
  const reportHandle = await fs.open(reportPath, 'wx', 0o600);
  const report = { passed: false, platform: process.platform, os: os.release(), arch: process.arch,
    driver: { pid: process.pid, node: process.version }, permissionSource,
    source: { entry: 'electron/main.cjs', preload: 'electron/preload.cjs', renderer: 'dist/index.html' },
    scope: 'Native Storage only; no recording, audio, permission or TCC changes', cleanup: {} };
  const deadline = Date.now() + 95000;
  const budget = (cap = 5000) => {
    const remaining = deadline - Date.now();
    assert.ok(remaining > 0, 'Native Storage overall deadline exceeded');
    return Math.min(cap, remaining);
  };
  let home, backend, application, electronChild, page, logHandle, failure;
  const sleep = () => new Promise(resolve => setTimeout(resolve, 150));
  const poll = async (fn, label, cap = 8000) => {
    const until = Date.now() + budget(cap);
    let last;
    do {
      try { return await fn(); } catch (error) { last = error; }
      await sleep();
    } while (Date.now() < until);
    throw new Error(`${label}: ${last?.message ?? 'not ready'}`, { cause: last });
  };
  let env;
  const osa = async (script, args = []) => {
    const { stdout } = await run('/usr/bin/osascript', ['-e', script, ...args], {
      env, timeout: budget(3500), killSignal: 'SIGKILL', maxBuffer: 32768,
    });
    return stdout.trim();
  };
  const panel = (operation) => osa(panelScript, [String(electronChild.pid), home, operation]);
  const snapshot = async () => {
    const runtime = await within(page.evaluate(() => window.apparent.readRuntimeState()), budget(), 'Runtime read');
    assert.deepEqual(runtime, { status: 'connected', phase: 'idle' });
    const location = await within(page.evaluate(() => window.apparent.readCaptureLocation()), budget(), 'Storage read');
    assert.equal(location.status, 'connected');
    assert.equal(location.storage.revision, undefined, 'Renderer must not receive revisions');
    return { runtime, location };
  };
  try {
    report.source.commit = (await run('/usr/bin/git', ['rev-parse', 'HEAD'], {
      cwd: root, timeout: budget(), maxBuffer: 4096,
    })).stdout.trim();
    report.source.sha256 = {};
    for (const file of ['electron/main.cjs', 'electron/preload.cjs', 'electron/runtime.cjs', 'src/storage.tsx', 'dist/index.html']) {
      report.source.sha256[file] = createHash('sha256').update(await fs.readFile(path.join(root, file))).digest('hex');
    }
    home = await fs.mkdtemp(path.join(runner, 'sn-'));
    await fs.chmod(home, 0o700);
    report.fixtureHome = home;
    const socket = path.join(home, '.config/apparent/agent-control/capture.sock');
    assert.ok(Buffer.byteLength(socket) <= 103, `Socket path too long (${Buffer.byteLength(socket)} bytes)`);
    const config = path.join(home, '.config/apparent/config.json');
    const selected = path.join(home, 'selected captures');
    const builtin = path.join(home, 'Library/Application Support/apparent/captures');
    const sentinel = path.join(builtin, 'existing-file.txt');
    const sentinelText = 'synthetic existing capture; do not move or remove\n';
    const unknown = { marker: 'synthetic-only', nested: { preserved: true } };
    await fs.mkdir(path.dirname(config), { recursive: true, mode: 0o700 });
    await fs.mkdir(selected, { mode: 0o700 });
    await fs.mkdir(builtin, { recursive: true, mode: 0o700 });
    await fs.writeFile(sentinel, sentinelText, { mode: 0o600, flag: 'wx' });
    await fs.writeFile(config, JSON.stringify({ experiments: { electronCaptureLocation: true },
      onboarding: { seen: true }, storageNativeFixture: unknown }), { mode: 0o600, flag: 'wx' });
    env = { ...process.env, HOME: home, APPARENT_CONFIG: config, CFFIXED_USER_HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'),
      TMPDIR: home, TMP: home, TEMP: home };
    delete env.APPARENT_CAPTURES_DIR;
    // The driver may have been started using an Electron binary in Node mode.
    // GUI children must always be normal Electron, never ELECTRON_RUN_AS_NODE.
    delete env.ELECTRON_RUN_AS_NODE;
    assert.equal(await osa('tell application "System Events" to get UI elements enabled'), 'true',
      'Runner Accessibility/AppleEvents preflight unavailable; do not edit TCC');
    report.accessibilityPreflight = true;
    logHandle = await fs.open(path.join(output, 'rust-storage-native.log'), 'wx', 0o600);
    const pidFile = path.join(home, 'rust.pid');
    await fs.writeFile(pidFile, '', { mode: 0o600, flag: 'wx' });
    backend = spawn('/bin/bash', ['-c', rustSupervisor, 'storage-native-supervisor', pidFile, executable, 'app'], {
      env, detached: true, stdio: ['pipe', logHandle.fd, logHandle.fd],
    });
    let spawnError;
    backend.once('error', error => { spawnError = error; });
    report.rust = { supervisorPid: backend.pid, executable, argument: 'app',
      executableSha256: createHash('sha256').update(await fs.readFile(executable)).digest('hex'),
      helperSha256: createHash('sha256').update(await fs.readFile(helperExecutable)).digest('hex') };
    await poll(async () => {
      if (spawnError) throw spawnError;
      assert.equal(backend.exitCode, null, 'Installed Rust GUI exited before readiness');
      assert.ok((await fs.lstat(socket)).isSocket());
      // The config's `running` record is Linux-only. The owned supervisor is
      // the Mac PID producer; verify ancestry and the actual mapped executable.
      const pidText = await fs.readFile(pidFile, 'utf8');
      assert.match(pidText, /^[1-9][0-9]{0,9}\n$/);
      const pid = Number(pidText);
      assert.ok(Number.isSafeInteger(pid) && pid !== backend.pid);
      process.kill(pid, 0);
      const relation = await run('/bin/ps', ['-p', String(pid), '-o', 'ppid=', '-o', 'pgid='],
        { env, timeout: budget(1500), maxBuffer: 4096 });
      assert.deepEqual(relation.stdout.trim().split(/\s+/).map(Number), [backend.pid, backend.pid]);
      const mapped = await run('lsof', ['-nP', '-a', '-p', String(pid), '-d', 'txt', '-Fn'],
        { env, timeout: budget(2500), maxBuffer: 131072 });
      const paths = mapped.stdout.split(/\r?\n/).filter(line => line.startsWith('n/')).map(line => line.slice(1));
      const actual = await Promise.all(paths.map(value => fs.realpath(value).catch(() => value)));
      assert.ok(actual.includes(await fs.realpath(executable)), 'Owned Rust PID does not map the installed executable');
      Object.assign(report.rust, { pid, parentAndGroupVerified: true, mappedExecutableVerified: true });
    }, 'Installed Rust socket readiness', 15000);
    application = await electron.launch({ args: [root], env, timeout: budget(15000) });
    electronChild = application.process();
    // launch() attaches to the main process before app.whenReady() creates a window.
    page = await application.firstWindow({ timeout: budget() });
    report.electron = await within(application.evaluate(({ app, BrowserWindow }) => ({
      pid: process.pid, version: process.versions.electron, platform: process.platform,
      home: process.env.HOME, userData: app.getPath('userData'),
      windowCount: BrowserWindow.getAllWindows().length,
      title: BrowserWindow.getAllWindows()[0].getTitle(),
    })), budget(), 'Electron process identity');
    assert.equal(report.electron.pid, electronChild.pid);
    assert.equal(report.electron.home, home);
    assert.ok(report.electron.userData.startsWith(home + path.sep), 'Electron profile escaped private HOME');
    assert.equal(report.electron.platform, 'darwin');
    assert.equal(report.electron.windowCount, 1);
    page.setDefaultTimeout(5000);
    const rendererErrors = [];
    page.on('pageerror', error => rendererErrors.push(error.message));
    await page.getByText('Connected · idle', { exact: true }).waitFor({ timeout: budget(10000) });
    const card = page.getByRole('region', { name: 'Capture location', exact: true });
    const change = card.getByRole('button', { name: 'Change…', exact: true });
    const reset = card.getByRole('button', { name: 'Reset', exact: true });
    const assertFiles = async (savedPath) => {
      const raw = JSON.parse(await fs.readFile(config, 'utf8'));
      if (savedPath === null) assert.equal(Object.hasOwn(raw, 'capturesDir'), false);
      else assert.equal(raw.capturesDir, savedPath);
      assert.deepEqual(raw.storageNativeFixture, unknown);
      assert.equal(raw.experiments.electronCaptureLocation, true);
      assert.equal(await fs.readFile(sentinel, 'utf8'), sentinelText);
      await assert.rejects(fs.stat(path.join(selected, 'existing-file.txt')), { code: 'ENOENT' });
      return { capturesDir: savedPath, keyAbsent: !Object.hasOwn(raw, 'capturesDir'),
        unknownFieldsPreserved: true, experimentPreserved: true, existingFileUnchanged: true, existingFileNotMoved: true };
    };
    report.before = { ...await poll(snapshot, 'Initial authoritative read'), files: await assertFiles(null) };
    assert.equal(report.before.location.storage.source, 'builtin');
    assert.equal(report.before.location.storage.effectivePath, builtin);
    await card.getByText(report.before.location.storage.displayPath, { exact: true }).waitFor({ timeout: budget() });
    await change.click({ timeout: budget() });
    report.nativeBeforeGo = await poll(() => panel('inspect'), 'Native Change picker');
    // Only the OS picker receives the path. Never pass it through renderer/IPC.
    await osa(panelScript, [String(electronChild.pid), selected, 'go']);
    report.nativeBeforeChoose = await poll(() => panel('inspect'), 'Picker after Go to Folder');
    // Optional evidence: bounded rectangle of our verified Electron window only.
    const bounds = report.nativeBeforeChoose.match(/bounds=(-?\d+),(-?\d+),(\d+),(\d+)$/);
    if (bounds && bounds.slice(1).every(v => Number.isSafeInteger(Number(v))) && Number(bounds[3]) > 0 && Number(bounds[4]) > 0) {
      try {
        await run('/usr/sbin/screencapture', ['-x', '-R', bounds.slice(1).join(','), path.join(output, 'native-picker.png')],
          { env, timeout: budget(3000), killSignal: 'SIGKILL', maxBuffer: 8192 });
        report.nativePickerScreenshot = 'native-picker.png';
      } catch (error) { report.nativePickerScreenshotUnavailable = error.message; }
    }
    await panel('choose');
    const changed = await poll(async () => {
      const next = await snapshot();
      assert.equal(next.location.storage.savedPath, selected);
      return next;
    }, 'Saved location readback');
    await card.getByText(changed.location.storage.displayPath, { exact: true }).waitFor({ timeout: budget() });
    report.change = { ...changed, files: await assertFiles(selected) };
    assert.equal(report.change.location.storage.source, 'configured');
    assert.equal(report.change.location.storage.savedPath, selected);
    assert.equal(report.change.location.storage.effectivePath, selected);
    await page.screenshot({ path: path.join(output, 'storage-saved.png'), timeout: budget() });
    const beforeCancel = await fs.readFile(config);
    const beforeCancelStat = await fs.stat(config, { bigint: true });
    await change.click({ timeout: budget() });
    report.nativeBeforeCancel = await poll(() => panel('inspect'), 'Reopened native picker');
    await panel('cancel');
    await expect(change).toBeEnabled({ timeout: budget() });
    assert.deepEqual(await fs.readFile(config), beforeCancel, 'Cancel changed config bytes');
    const afterCancelStat = await fs.stat(config, { bigint: true });
    assert.equal(afterCancelStat.ino, beforeCancelStat.ino, 'Cancel replaced config');
    assert.equal(afterCancelStat.mtimeNs, beforeCancelStat.mtimeNs, 'Cancel rewrote config');
    report.cancel = { ...await snapshot(), files: await assertFiles(selected),
      bytesUnchanged: true, inodeUnchanged: true, mtimeUnchanged: true };
    assert.deepEqual(report.cancel.location, report.change.location);
    await reset.click({ timeout: budget() });
    const resetRead = await poll(async () => {
      const next = await snapshot();
      assert.equal(next.location.storage.savedPath, null);
      return next;
    }, 'Reset location readback');
    await card.getByText(resetRead.location.storage.displayPath, { exact: true }).waitFor({ timeout: budget() });
    await expect(reset).toBeDisabled({ timeout: budget() });
    report.reset = { ...resetRead, files: await assertFiles(null) };
    assert.equal(report.reset.location.storage.savedPath, null);
    assert.equal(report.reset.location.storage.source, 'builtin');
    assert.equal(report.reset.location.storage.effectivePath, builtin);
    await page.screenshot({ path: path.join(output, 'storage-reset.png'), timeout: budget() });
    assert.deepEqual(rendererErrors, []);
    report.passed = true;
  } catch (error) {
    failure = error;
    report.error = { message: error.message, stack: error.stack,
      subprocess: error.cause && { code: error.cause.code, signal: error.cause.signal,
        killed: error.cause.killed, stderr: error.cause.stderr } };
    if (electronChild && Date.now() < deadline) {
      try { report.failureAX = await panel('diagnose'); } catch (axError) { report.failureAXError = axError.message; }
      // Read-only evidence from the exact live owned window, even when its native
      // controls cannot be selected. Never expand to the display or other windows.
      if (application && Date.now() < deadline) {
        try {
          const bounds = await within(application.evaluate(({ BrowserWindow }) => {
            const windows = BrowserWindow.getAllWindows();
            if (windows.length !== 1 || windows[0].getTitle() !== 'Apparent — UI foundation') throw new Error('Unknown owned window');
            return windows[0].getBounds();
          }), budget(), 'Owned failure window bounds');
          const rectangle = [bounds.x, bounds.y, bounds.width, bounds.height];
          assert.ok(rectangle.every(Number.isSafeInteger) && bounds.width > 0 && bounds.height > 0);
          await run('/usr/sbin/screencapture', ['-x', '-R', rectangle.join(','), path.join(output, 'failed-owned-window.png')],
            { env, timeout: budget(3000), killSignal: 'SIGKILL', maxBuffer: 8192 });
          report.failureScreenshot = 'failed-owned-window.png';
        } catch (shotError) { report.failureScreenshotUnavailable = shotError.message; }
      }
    }
  } finally {
    if (application) {
      try { await within(application.close(), 3000, 'Electron close'); }
      catch (error) { report.cleanup.electronCloseError = error.message; }
    }
    for (const [label, child, group] of [['electron', electronChild, false], ['rust', backend, true]]) {
      try { report.cleanup[label] = await stopOwned(child, label, group); }
      catch (error) { report.cleanup[label] = { error: error.message }; failure ??= error; report.passed = false; }
    }
    if (logHandle) await logHandle.close();
    // Keep failed fixtures and evidence. A successful run removes only its own HOME.
    if (home && !failure) {
      try { await fs.rm(home, { recursive: true, force: true }); report.cleanup.fixtureRemoved = true; }
      catch (error) { report.cleanup.fixtureError = error.message; failure = error; report.passed = false; }
    } else report.cleanup.fixtureRetained = home ?? null;
    report.cleanup.limit = 'Owned process/group termination only; no global process or permission cleanup';
    await reportHandle.writeFile(JSON.stringify(report, null, 2) + '\n');
    await reportHandle.close();
  }
  console.log(`Native Storage evidence: ${reportPath}`);
  if (failure) throw failure;
});

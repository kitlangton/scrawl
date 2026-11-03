// Installed completed-capture public-copy proof against local Worker/D1/R2 only.
// No Google, production account, real capture, or browser-cookie proof.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn, spawnSync} from 'node:child_process';
import {mkdir, mkdtemp, readFile, writeFile, readdir, realpath, rm, cp} from 'node:fs/promises';
import {openSync, closeSync, existsSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {captureFailureReceipt} from './share-receipt.ts';
assert.equal(process.env.APPARENT_CONNECT_NATIVE_CI, '1', 'Explicit native proof opt-in required');
assert.equal(process.platform, 'darwin', 'This case uses the hosted Mac owned-window driver');
if (process.platform === 'darwin') {
  assert.equal(process.env.GITHUB_ACTIONS, 'true');
  assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted', 'Never run on a personal Mac');
} else {
  assert.equal(process.env.APPARENT_CONNECT_NATIVE_ISOLATED_LINUX, '1');
  assert.equal(spawnSync('systemd-detect-virt', ['--vm']).status, 0, 'Use the isolated Linux VM recipe');
}
process.umask(0o077);
const [sourceArg, binaryArg, outputArg] = process.argv.slice(2);
assert(sourceArg && binaryArg && outputArg, 'source, installed binary and fresh output required');
const source = await realpath(sourceArg), binary = await realpath(binaryArg);
assert(process.platform === 'darwin' ? binary.endsWith('.app/Contents/MacOS/apparent') : binary.endsWith('/package/bin/apparent'), 'Use the installed bundle or isolated-VM prefix');
const output = resolve(outputArg);
await mkdir(output, {mode: 0o700});
const home = await realpath(await mkdtemp('/tmp/acn-')), evidence = join(output, 'evidence');
await mkdir(evidence, {mode: 0o700});
process.chdir(join(source, 'connect/service'));
let server, service, account, desktop, log, scene, recorder, capturedBundle, runDiagnosticUpload;
const children = new Set(), calls = [], desktopPids = [];
const report: {passed:boolean;scope:string;[key:string]:unknown} = {passed: false,
  scope: 'installed Mac completed-controls public-copy; synthetic hosted desktop and loopback Worker; no Google/deployed proof'};
const delay = (ms:number) => new Promise<void>(r => setTimeout(r, ms));
const alive = p => p.exitCode === null && p.signalCode === null;
async function stop(p) {
  if (!p) return;
  if (!Number.isInteger(p.pid)) { children.delete(p); return; }
  if (alive(p)) {
    const closed = new Promise(r => p.once('close', r));
    process.kill(-p.pid, 'SIGTERM');
    let timer;
    try { await Promise.race([closed, new Promise<void>(r => { timer = setTimeout(() => {
      if (alive(p)) process.kill(-p.pid, 'SIGKILL'); r();
    }, 5000); })]); await closed; } finally { clearTimeout(timer); }
  }
  children.delete(p);
}
async function retainBundle(bundle) {
  // Artifact allowlist is independent of upload admission. Never retain UI diagnostics.
  const target = join(evidence, 'synthetic.apparent'); await mkdir(target, {mode: 0o700});
  for (const name of await readdir(bundle)) {
    if (/^(apparent\.json|notes\.json|summary\.md|contact-sheet(?:-full)?\.png|recording\.(mp4|mov)|audio\.wav|timeline\.json|finish-trace\.json)$/.test(name))
      await cp(join(bundle, name), join(target, name));
    else if (name === 'frames' || name === 'frames-full') {
      await mkdir(join(target, name), {mode: 0o700});
      for (const file of await readdir(join(bundle, name))) if (/^frame-[0-9]+\.png$/.test(file)) await cp(join(bundle, name, file), join(target, name, file));
    }
  }
}
try {
// Load platform-native Worker dependencies after establishing failure evidence.
const {fixture} = await import('../helpers.mjs');
server = createServer(async (req, res) => {
  try {
    assert(req.url.startsWith('/v1/') || req.url.startsWith('/s/'), 'Only local native API/public routes admitted');
    const receipt: {method:string;path:string;status?:number} = {method: req.method, path: new URL(req.url, 'http://fixture').pathname}; calls.push(receipt);
    const chunks = []; let bytes = 0;
    for await (const chunk of req) { bytes += chunk.length; assert(bytes < 4 * 1024 * 1024); chunks.push(chunk); }
    const response = await service.mf.dispatchFetch(origin + req.url, {method: req.method, headers: req.headers,
      ...(chunks.length ? {body: Buffer.concat(chunks)} : {})});
    receipt.status = response.status;
    res.writeHead(response.status, Object.fromEntries(response.headers)); res.end(Buffer.from(await response.arrayBuffer()));
  } catch { res.writeHead(500); res.end('{"error":{"code":"fixture_error"}}'); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`, config = join(home, 'config.json');
await writeFile(config, JSON.stringify({microphoneEnabled: false, experiments: {hostedEncryption: true, hostedSync: true, hostedSharing: true}, cloud: {server: origin, enabled: false, uploadConsent: false},
  onboarding: {seen: true}, errorReporting: false, syntheticFixture: {preserve: true}}), {mode: 0o600});
const env:NodeJS.ProcessEnv = {...process.env, HOME: home, CFFIXED_USER_HOME: home, XDG_CONFIG_HOME: join(home, '.config'),
  TMPDIR: home, TMP: home, TEMP: home, APPARENT_CONFIG: config, APPARENT_CAPTURES_DIR: join(home, 'captures')};
delete env.ELECTRON_RUN_AS_NODE;
// Keep model/update curl invocations offline; native transport may only reach this fixture.
const bin = join(home, 'bin'); await mkdir(bin, {mode: 0o700});
await writeFile(join(bin, 'curl'), `#!/usr/bin/env python3
import sys,subprocess,re,json,os
args=sys.argv[1:]
booleans={'--disable','--silent','--show-error','--no-netrc'}
values={'--proto','--max-redirs','--connect-timeout','--max-time','--max-filesize','--output','--write-out','--config'}
seen={};i=0
while i<len(args):
 flag=args[i]
 if flag in seen or flag not in booleans|values: sys.exit(22)
 if flag in booleans: seen[flag]=True;i+=1
 else:
  if i+1==len(args): sys.exit(22)
  seen[flag]=args[i+1];i+=2
if not booleans.issubset(seen) or seen.get('--write-out')!='%{http_code}' or seen.get('--config')!='-' or seen.get('--max-redirs')!='0' or seen.get('--proto')!='=http,https': sys.exit(22)
data=sys.stdin.buffer.read(65537)
if len(data)>65536: sys.exit(22)
if any(line.split(b' = ',1)[0] not in {b'url',b'request',b'header',b'data-binary'} for line in data.splitlines()): sys.exit(22)
urls=re.findall(rb'^url = (".*")$',data,re.M)
if len(urls)!=1 or not json.loads(urls[0]).startswith(${JSON.stringify(origin + '/v1/')}): sys.exit(22)
env={k:v for k,v in os.environ.items() if not k.lower().endswith('_proxy')}
env.update(NO_PROXY='*',no_proxy='*')
sys.exit(subprocess.run(['/usr/bin/curl',*args],input=data,env=env).returncode)
`, {mode: 0o700});
env.PATH = bin + ':' + env.PATH;
for (const args of [['https://example.invalid'], ['--proxy', 'http://example.invalid', '--config', '-'], ['--location', '--config', '-'], ['--resolve', '127.0.0.1:80:192.0.2.1', '--config', '-']])
  assert.equal(spawnSync(join(bin, 'curl'), args, {env, input: ''}).status, 22);
function cli(args:string[], onData:(text:string)=>void = () => {}):Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const p = spawn(binary, ['fetch', 'capture'].includes(args[0]) ? args : ['connect', ...args], {env, detached: true, stdio: ['ignore', 'pipe', 'pipe']});
    children.add(p); let stdout = '';
    let hardTimeout;
    const timeout = setTimeout(() => { if (alive(p)) { process.kill(-p.pid, 'SIGTERM');
      hardTimeout = setTimeout(() => { if (alive(p)) process.kill(-p.pid, 'SIGKILL'); }, 5000); } }, 80000);
    p.stdout.on('data', b => { stdout += b; if (stdout.length > 65536) p.kill('SIGTERM'); else onData(stdout); });
    p.stderr.resume(); // Native errors can contain paths; expose only the operation and exit below.
    p.once('error', () => { clearTimeout(timeout); clearTimeout(hardTimeout); children.delete(p); reject(new Error(`Native ${args[0]} could not start`)); });
    p.once('close', code => { clearTimeout(timeout); clearTimeout(hardTimeout); children.delete(p);
      if (code === 0) resolve(stdout); else {
        if (args[0] === 'capture') report.captureFailure = captureFailureReceipt(args[1], stdout);
        reject(new Error(`Native ${args[0]} failed (${code})`));
      } });
  });
}
runDiagnosticUpload = bundle => cli(['upload', bundle]);
async function login(scope) {
  let approval;
  await cli(['login', '--server', origin, '--scope', scope, ...(scope === 'upload' ? ['--consent-upload'] : []), '--no-browser'], text => {
    const code = /Verification code: ([A-Za-z0-9-]+)/.exec(text)?.[1];
    if (!code || approval) return;
    approval = fetch(origin + '/v1/device/approve', {method: 'POST', headers: {Cookie: account.cookie, Origin: origin,
      'X-CSRF-Token': account.csrf, 'Content-Type': 'application/json'}, body: JSON.stringify({userCode: code})})
      .then(r => assert.equal(r.status, 200));
    approval.catch(() => {});
  });
  assert(approval, 'Native approval was not requested'); await approval;
}
async function startDesktop() {
  log ??= openSync(join(evidence, 'desktop.log'), 'wx', 0o600);
  desktop = spawn(binary, ['app'], {env, detached: true, stdio: ['ignore', log, log]}); children.add(desktop); desktopPids.push(desktop.pid);
  const deadline = Date.now() + 20000;
  while (!existsSync(join(home, '.config/apparent/agent-control/capture.sock'))) {
    assert(alive(desktop), 'Desktop exited'); assert(Date.now() < deadline, 'Desktop readiness timeout'); await delay(100);
  }
  await delay(500); assert(alive(desktop), 'Desktop exited after readiness');
}
const driver = join(output, 'share-driver');
const compiled = spawnSync('swiftc', [join(source, 'capture/desktop/scripts/native/public-share.swift'), '-o', driver], {timeout: 45000, encoding: 'utf8'});
assert.equal(compiled.status, 0, 'Native driver compilation failed: ' + compiled.stderr);
let serial = 0;
function ui(action:string, pid?:number|string, label?:string) {
  const args = action === 'clipboard' ? [action, ...(pid === undefined ? [] : [String(pid)])] : [action, String(pid), label, join(evidence, `window-${++serial}.png`)];
  const result = spawnSync(driver, args, {env, timeout: 12000, encoding: 'utf8'});
  if (action === 'click') {
    const diagnostics = result.stdout.split('\n').filter(line => line.startsWith('{')).map(line => JSON.parse(line));
    report.nativeInput = [...(report.nativeInput as unknown[] ?? []), ...diagnostics];
  }
  assert.equal(result.status, 0, `Owned-window ${action} failed: ${result.stderr}`);
  return result.stdout.trim();
}
async function until(probe, seconds, message) {
  const end = Date.now() + seconds * 1000;
  do { const result = await probe(); if (result) return result; await delay(200); } while (Date.now() < end);
  throw Error(message);
}
function installedChildPID(surface: 'pill' | 'share' | 'notes') {
  const pids = spawnSync('/usr/bin/pgrep', ['-P', String(desktop.pid)], {encoding: 'utf8'}).stdout.trim().split(/\s+/).filter(Boolean);
  const expected = binary.slice(0, binary.indexOf('.app/') + 5);
  const matches = pids.filter(pid => {
    const argv = spawnSync('/bin/ps', ['-ww', '-p', pid, '-o', 'command='], {encoding: 'utf8'}).stdout;
    return argv.includes(expected) && argv.includes(`/electron/${surface}/main.cjs`);
  });
  assert(matches.length <= 1, `Multiple installed ${surface} children`); return matches.length ? Number(matches[0]) : null;
}
const popupPID = () => installedChildPID('share');

async function openPopup() {
  // Shared pill startup failure deliberately restores the native completed
  // controls. Exercise that installed entry too; the consent child below must
  // still be the packaged Electron share surface, never a direct-copy fallback.
  const pill = installedChildPID('pill');
  report.completedEntrySurface = pill ? 'electron-pill' : 'native-controls';
  ui('click', pill ?? desktop.pid, 'Copy link');
  const pid = await until(popupPID, 10, 'Installed share child absent');
  // OCR is scoped to this child; it proves the actual installed renderer/disclosure.
  await until(() => { try { ui('inspect', pid, 'not end-to-end encrypted'); return true; } catch { return false; } }, 8, 'Explicit disclosure absent');
  return pid;
}
  service = await fixture({API_ORIGIN: origin, SITE_ORIGIN: origin, SHARING_ENABLED: 'true'});
  await service.db.exec((await readFile('migrations/0003.sql', 'utf8')).replaceAll('\n', ' '));
  account = await service.identity('native-public-share-synthetic');
  await login('read'); await login('upload');
  scene = spawn(driver, ['scene'], {env, detached: true, stdio: 'ignore'}); children.add(scene);
  await delay(1000); assert(alive(scene)); await startDesktop();
  const id = createHash('sha256').update(output).digest('hex').slice(0, 32);
  const started = JSON.parse(await cli(['capture', 'start', '--request-id', id, '--wait', '--timeout', '20']));
  assert.equal(started.capture_id, id); assert.equal(started.status, 'recording');
  // Capture admission precedes asynchronous notes renderer startup. Do not
  // spend the unchanged six-second finish barrier on cold renderer startup.
  await until(() => {
    const notes = installedChildPID('notes');
    if (!notes) return false;
    try { ui('inspect', notes, 'Notes'); return true; } catch { return false; }
  }, 30, 'Installed notes renderer not ready before capture stop');
  const recording = JSON.parse(await cli(['capture', 'inspect', id]));
  assert.equal(recording.capture_id, id); assert.equal(recording.status, 'recording');
  report.notesReadyBeforeStop = true;
  await delay(2500);
  const saved = JSON.parse(await cli(['capture', 'stop', id, '--wait', '--timeout', '60']));
  assert.equal(saved.capture_id, id); assert.equal(saved.status, 'saved');
  const bundle = saved.bundle_path; assert(bundle.startsWith(join(home, 'captures') + '/')); capturedBundle = bundle;
  Object.assign(report, {captureSaved: true, binarySHA256: createHash('sha256').update(await readFile(binary)).digest('hex')});
  await retainBundle(bundle);
  const known = new Set(['apparent.json', 'notes.json', 'summary.md', 'contact-sheet.png', 'contact-sheet-full.png', 'recording.mp4', 'recording.mov', 'audio.wav', 'timeline.json', 'finish-trace.json', 'frames', 'frames-full', 'notes-ui.log', '.publication.lock', '.publication-pending', 'unfinished.json']);
  const {lstat} = await import('node:fs/promises');
  report.sourceEntries = await Promise.all((await readdir(bundle)).map(async name => {
    const stat = await lstat(join(bundle, name));
    return {name: known.has(name) ? name : 'unrecognized', regular: stat.isFile(), directory: stat.isDirectory(),
      symlink: stat.isSymbolicLink(), owned: stat.uid === process.getuid(), mode: stat.mode & 0o777, links: stat.nlink, size: stat.size};
  }));
  const acl = spawnSync('/bin/ls', ['-lde', home, join(home, 'captures'), bundle, join(bundle, 'frames')], {encoding: 'utf8', timeout: 3000});
  report.sourceAcl = {checked: acl.status === 0, hasAllowEntry: /\ballow\b/.test(acl.stdout || '')};
  const jobs = async () => Promise.all((await readdir(join(home, 'hosted'))).filter(n => n.endsWith('.job')).map(async n => JSON.parse(await readFile(join(home, 'hosted', n), 'utf8'))));
  const privateJob = await until(async () => (await jobs()).find(j => j.encrypted && j.status === 'ready'), 25, 'Approved private transfer not ready');
  const privateFile = join(home, 'hosted', privateJob.id + '.job');
  const privateBytes = await readFile(privateFile);
  recorder = spawn('/usr/sbin/screencapture', ['-v', '-x', '-V', '110', join(evidence, 'public-copy.mov')], {env, detached: true, stdio: 'ignore'});
  children.add(recorder);
  await delay(1000);
  ui('clipboard', 'synthetic sentinel');
  let pid = await openPopup();
  const beforeCancel = (await jobs()).map(j => j.id).sort();
  // Longer than the completed pill's 25-second expiry: close must grant retry time.
  await delay(26000);
  ui('click', pid, 'Cancel');
  await until(() => !popupPID(), 5, 'Cancelled child remains');
  assert.deepEqual((await jobs()).map(j => j.id).sort(), beforeCancel);
  assert.equal(ui('clipboard'), 'synthetic sentinel');
  async function copy() {
    const child = await openPopup();
    ui('click', child, 'Create and copy public link');
    const text = await until(() => { const text = ui('clipboard'); return text.includes('/s/') ? text : null; }, 25, 'Native clipboard did not receive public link');
    ui('inspect', child, 'Link copied');
    ui('click', child, 'Close'); await until(() => !popupPID(), 5, 'Copied child remains');
    return text;
  }
  const copied = await copy(), url = copied.match(/http:\/\/127\.0\.0\.1:\d+\/s\/[a-f0-9]{64}/)?.[0];
  assert(url, 'Clipboard lacks local capability');
  ui('clipboard', 'repeat sentinel'); assert.equal(await copy(), copied);
  assert((await readFile(privateFile)).equals(privateBytes), 'Public copy changed private job');
  const manifest = await until(async () => { const r = await fetch(url + '/manifest.json'); const value = await r.json(); return value.status === 'ready' ? value : null; }, 25, 'Anonymous recording not ready');
  const digest = b => createHash('sha256').update(b).digest('hex');
  for (const file of manifest.files) {
    assert(!file.path.startsWith('/') && !file.path.split('/').includes('..'));
    const r = await fetch(file.url); assert(r.ok);
    const bytes = Buffer.from(await r.arrayBuffer());
    assert.equal(digest(bytes), file.sha256); assert(bytes.equals(await readFile(join(bundle, file.path))));
  }
  const publicJob = (await jobs()).find(j => j.publicCopy); assert(publicJob && !publicJob.encrypted && publicJob.id !== privateJob.id);
  const share = await service.db.prepare('SELECT id FROM shares WHERE capture_id = ?').bind(publicJob.id).first(); assert(share);
  assert.equal((await service.call('/v1/shares/' + share.id, {token: account.token, method: 'DELETE'})).status, 200);
  assert.equal((await fetch(url + '/manifest.json')).status, 410);
  const fetched = join(home, 'decrypted.apparent'); await cli(['fetch', privateJob.id, '--out', fetched]);
  assert((await readFile(join(fetched, 'apparent.json'))).equals(await readFile(join(bundle, 'apparent.json'))));
  Object.assign(report, {passed: true, captureSaved: true, installedPopup: true, explicitConsent: true, cancelNoPublication: true,
    repeatedNativeClipboard: true, completedRetryAfterLongPopup: true, originalAssetsMatch: true, revoked: true, privateEncryptedPreserved: true,
    binarySHA256: digest(await readFile(binary)), sourceCommit: spawnSync('git', ['-C', source, 'rev-parse', 'HEAD'], {encoding: 'utf8'}).stdout.trim()});
} catch (error) { report.failure = error.message.replace(/\/s\/[a-f0-9]{64}/g, '/s/[redacted]'); process.exitCode = 1; }
finally {
  const cleanupErrors = [];
  // Only allowlisted status/error enums and numeric facts survive failed preparation.
  try {
    const allowedErrors = new Set(['file_too_large', 'encryption_pending_recipient', 'encryption_action_required', 'quota_exceeded', 'unsupported_asset', 'auth_required', 'upload_failed']);
    report.uploadQueue = await Promise.all((await readdir(join(home, 'hosted'))).filter(n => n.endsWith('.job')).map(async name => {
      const j = JSON.parse(await readFile(join(home, 'hosted', name), 'utf8'));
      return {status: ['pending', 'ready', 'failed', 'cancelled', 'expired'].includes(j.status) ? j.status : 'unknown',
        error: j.error == null ? null : allowedErrors.has(j.error) ? j.error : 'unknown',
        encrypted: j.encrypted === true, publicCopy: j.publicCopy === true,
        attempts: Number.isSafeInteger(j.attempts) ? j.attempts : null};
    }));
    if (!report.passed && capturedBundle && report.failure === 'Approved private transfer not ready') {
      // A failed case stays failed. Stop the GUI worker before the explicit
      // synthetic CLI control so another worker cannot own that control upload.
      try {
        await stop(desktop); report.sameSourceCliGuiStopped = !alive(desktop);
        assert(report.sameSourceCliGuiStopped, 'GUI must stop before the CLI diagnostic');
        report.sameSourceCliAfterApiCall = calls.length;
        await runDiagnosticUpload(capturedBundle);
        const controls = await Promise.all((await readdir(join(home, 'hosted'))).filter(n => n.endsWith('.job')).map(async n => JSON.parse(await readFile(join(home, 'hosted', n), 'utf8'))));
        report.sameSourceCliEncryptedReady = controls.some(j => j.bundle === capturedBundle && j.encrypted && j.status === 'ready');
      } catch { report.sameSourceCliControlFailed = true; }
    }
    report.apiStatuses = calls.map(c => ({route: c.path === '/v1/encryption/peers' ? 'encryption_peers' : c.path === '/v1/account' ? 'reachability' : 'other', method: c.method, status: c.status ?? null}));
  } catch { report.queueInspectionUnavailable = true; }

  if (recorder && alive(recorder)) {
    recorder.kill('SIGINT');
    await Promise.race([new Promise(r => recorder.once('close', r)), delay(5000)]);
  }
  if (recorder && !alive(recorder)) {
    const video = join(evidence, 'public-copy.mov');
    report.videoDecodes = existsSync(video) && spawnSync('ffmpeg', ['-v', 'error', '-i', video, '-f', 'null', '-'], {timeout: 20000}).status === 0;
    if (!report.videoDecodes) { await rm(video, {force: true}); report.videoLimit = 'Native recorder did not produce decodable video; owned-window screenshots retained'; }
  }
  for (const child of [...children]) { try { await stop(child); } catch { cleanupErrors.push('owned child cleanup failed'); } }
  // A failed finalizer can still leave useful synthetic media. Preserve the
  // existing asset allowlist only after its writers stop, before deleting home.
  if (!report.passed && !capturedBundle && !cleanupErrors.length) {
    try {
      const entries = (await readdir(join(home, 'captures'))).filter(name => /^apparent-[0-9]+-[a-f0-9]+-[0-9]+\.apparent$/.test(name));
      if (entries.length === 1) {
        await retainBundle(join(home, 'captures', entries[0])); report.failedCaptureArtifactsRetained = true;
      }
    } catch { report.failedCaptureArtifactsUnavailable = true; }
  }
  if (log !== undefined) { try { closeSync(log); } catch { cleanupErrors.push('log close failed'); } }
  if (server?.listening) { server.closeAllConnections(); await new Promise(r => server.close(r)); }
  if (service) { try { await service.close(); } catch { cleanupErrors.push('local Worker cleanup failed'); } }
  if (cleanupErrors.length) { report.passed = false; report.cleanupErrors = cleanupErrors; process.exitCode = 1; }
  if (!cleanupErrors.length) {
    try { await rm(home, {recursive: true}); report.disposableHomeRemoved = true; }
    catch { report.passed = false; report.cleanupErrors = ['disposable home cleanup failed']; process.exitCode = 1; }
  }
  await writeFile(join(evidence, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
}

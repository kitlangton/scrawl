// Explicit installed-Desktop proof against actual local Worker/D1/R2 only.
// No Google, production account, real capture, or browser-cookie proof.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn, spawnSync} from 'node:child_process';
import {mkdir, mkdtemp, readFile, writeFile, readdir, realpath, rm} from 'node:fs/promises';
import {openSync, closeSync, existsSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {fixture} from '../helpers.mjs';
assert.equal(process.env.APPARENT_CONNECT_NATIVE_CI, '1', 'Explicit native proof opt-in required');
assert(['darwin', 'linux'].includes(process.platform));
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
let server, service, account, desktop, log, failMedia = false;
const children = new Set(), calls = [], desktopPids = [], report = {passed: false,
  scope: `installed ${process.platform} Desktop worker restart/retry; synthetic local identity/assets; no browser/Google/deployed proof`};
const delay = ms => new Promise(r => setTimeout(r, ms));
const alive = p => p.exitCode === null && p.signalCode === null;
async function stop(p) {
  if (!p) return;
  if (!Number.isInteger(p.pid)) { children.delete(p); return; }
  if (alive(p)) {
    const closed = new Promise(r => p.once('close', r));
    process.kill(-p.pid, 'SIGTERM');
    let timer;
    try { await Promise.race([closed, new Promise(r => { timer = setTimeout(() => {
      if (alive(p)) process.kill(-p.pid, 'SIGKILL'); r();
    }, 5000); })]); await closed; } finally { clearTimeout(timer); }
  }
  try { process.kill(-p.pid, 0); process.kill(-p.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  children.delete(p);
}
try {
server = createServer(async (req, res) => {
  try {
    assert(req.url.startsWith('/v1/'), 'Only local native API routes admitted');
    const receipt = {method: req.method, path: new URL(req.url, 'http://fixture').pathname}; calls.push(receipt);
    if (failMedia && req.method === 'PUT') {
      receipt.status = 503; res.writeHead(503, {'Content-Type': 'application/json'});
      res.end('{"error":{"code":"temporary_unavailable"}}'); return;
    }
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
await writeFile(config, JSON.stringify({experiments: {hostedEncryption: true}, cloud: {server: origin, enabled: false, uploadConsent: false},
  onboarding: {seen: true}, errorReporting: false, syntheticFixture: {preserve: true}}), {mode: 0o600});
const env = {...process.env, HOME: home, CFFIXED_USER_HOME: home, XDG_CONFIG_HOME: join(home, '.config'),
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
function cli(args, onData = () => {}) {
  return new Promise((resolve, reject) => {
    const p = spawn(binary, args[0] === 'fetch' ? args : ['connect', ...args], {env, detached: true, stdio: ['ignore', 'pipe', 'pipe']});
    children.add(p); let stdout = '';
    let hardTimeout;
    const timeout = setTimeout(() => { if (alive(p)) { process.kill(-p.pid, 'SIGTERM');
      hardTimeout = setTimeout(() => { if (alive(p)) process.kill(-p.pid, 'SIGKILL'); }, 5000); } }, 30000);
    p.stdout.on('data', b => { stdout += b; if (stdout.length > 65536) p.kill('SIGTERM'); else onData(stdout); });
    p.stderr.resume(); // Native errors can contain paths; expose only the operation and exit below.
    p.once('error', () => { clearTimeout(timeout); clearTimeout(hardTimeout); children.delete(p); reject(new Error(`Native ${args[0]} could not start`)); });
    p.once('close', code => { clearTimeout(timeout); clearTimeout(hardTimeout); children.delete(p);
      if (code === 0) resolve(stdout); else reject(new Error(`Native ${args[0]} failed (${code})`)); });
  });
}
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
const files = ['apparent.json', 'summary.md', 'contact-sheet.png', 'frames/frame-01.png', 'recording.mov', 'audio.wav'];
async function bundle(name) {
  const dir = join(home, 'captures', name + '.apparent'); await mkdir(join(dir, 'frames'), {recursive: true, mode: 0o700});
  await writeFile(join(dir, 'apparent.json'), JSON.stringify({video: 'recording.mov', audio: 'audio.wav', keyframes: [{file: 'frames/frame-01.png'}]}));
  await writeFile(join(dir, 'summary.md'), 'Synthetic Connect worker transfer.');
  for (const [target, asset] of [['contact-sheet.png', 'frame.png'], ['frames/frame-01.png', 'frame.png'], ['recording.mov', 'fixture.mov'], ['audio.wav', 'fixture.wav']])
    await writeFile(join(dir, target), await readFile(join(home, asset)));
  return dir;
}
async function jobs() { return Promise.all((await readdir(join(home, 'hosted'))).filter(n => n.endsWith('.job')).map(async n => JSON.parse(await readFile(join(home, 'hosted', n))))); }
  service = await fixture({API_ORIGIN: origin, SITE_ORIGIN: origin}); account = await service.identity('native-worker-synthetic');
  if (process.env.APPARENT_CONNECT_NATIVE_ASSETS) {
    for (const name of ['fixture.mov', 'frame.png', 'fixture.wav']) {
      const bytes = await readFile(join(process.env.APPARENT_CONNECT_NATIVE_ASSETS, name)); assert(bytes.length < 1024 * 1024);
      await writeFile(join(home, name), bytes);
    }
  } else {
  const media = spawnSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=gold:s=64x64:r=10', '-t', '0.2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', join(home, 'fixture.mov')], {timeout: 15000});
  assert.equal(media.status, 0, 'Synthetic movie generation failed');
  assert.equal(spawnSync('ffmpeg', ['-v', 'error', '-i', join(home, 'fixture.mov'), '-frames:v', '1', join(home, 'frame.png')], {timeout: 10000}).status, 0);
  const wav = Buffer.alloc(3244); wav.write('RIFF'); wav.writeUInt32LE(3236, 4); wav.write('WAVEfmt ', 8); wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22); wav.writeUInt32LE(16000, 24); wav.writeUInt32LE(32000, 28); wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write('data', 36); wav.writeUInt32LE(3200, 40);
  await writeFile(join(home, 'fixture.wav'), wav);
  }
  await bundle('historical'); await login('read');
  let settings = JSON.parse(await readFile(config)); assert.equal(settings.cloud.enabled, false); assert.equal(settings.cloud.uploadConsent, false);
  await startDesktop(); await stop(desktop); desktop = undefined; assert.equal((await jobs()).length, 0, 'Startup discovered history');
  await login('upload'); assert.equal((await jobs()).length, 0, 'Approval discovered history');
  const future = await bundle('future'); failMedia = true; await cli(['upload', future]);
  const before = (await jobs())[0]; assert.equal(before.status, 'pending'); assert.equal(before.error, 'upload_failed'); assert.equal(before.encrypted, true);
  const identity = JSON.stringify({id: before.id, files: before.files, bundle: before.bundle});
  failMedia = false; await startDesktop(); const deadline = Date.now() + 35000; let after;
  do { assert(alive(desktop)); after = (await jobs())[0]; if (after.status === 'ready') break;
    assert(Date.now() < deadline, 'Installed worker did not resume'); await delay(100); } while (true);
  assert.equal(JSON.stringify({id: after.id, files: after.files, bundle: after.bundle}), identity); assert(after.attempts > before.attempts);
  // No CLI retry/account/fetch runs until the installed worker has reached ready.
  const fetched = join(home, 'fetched.apparent'); await cli(['fetch', after.id, '--out', fetched]);
  const digest = b => createHash('sha256').update(b).digest('hex');
  for (const file of files) assert.equal(digest(await readFile(join(future, file))), digest(await readFile(join(fetched, file))));
  const objects = await service.bucket.list(); assert(objects.objects.length > 0);
  assert(objects.objects.every(o => !files.some(file => o.key.endsWith('/' + file))), 'Plain asset path reached storage');
  await cli(['disable']); await stop(desktop); desktop = undefined; await startDesktop();
  const requests = calls.length; await delay(2500); assert.equal(calls.length, requests, 'Disabled restart made API requests');
  await cli(['logout']); const snapshotRequests = calls.length; const status = JSON.parse(await cli(['status']));
  assert.equal(calls.length, snapshotRequests, 'Local status made API requests');
  assert.equal(status.readSignedIn, false); assert.equal(status.uploadSignedIn, false);
  settings = JSON.parse(await readFile(config)); assert.equal(settings.cloud.enabled, false); assert.equal(settings.cloud.uploadConsent, false);
  assert.deepEqual(settings.syntheticFixture, {preserve: true});
  Object.assign(report, {passed: true, files: files.length, attemptsBefore: before.attempts, attemptsAfter: after.attempts,
    failedPutCalls: calls.filter(c => c.status === 503).length, opaqueObjects: objects.objects.length, desktopPids,
    historyNotDiscovered: true, readApprovalDidNotEnableUploads: true, workerOwnedRetry: true, disabledRestartOffline: true, logoutSignedOut: true});
} catch (error) { report.failure = error.message; process.exitCode = 1; }
finally {
  const cleanupErrors = [];
  for (const child of [...children]) { try { await stop(child); } catch { cleanupErrors.push('owned child cleanup failed'); } }
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

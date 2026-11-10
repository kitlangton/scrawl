import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import test from 'node:test';

// Explicit integration lane: unlike owner-local tests, this requires a rebuilt
// native composition binary. Missing prerequisites fail rather than skip.
const binary = process.env.APPARENT_CONNECT_TEST_BINARY;
if (!binary || !path.isAbsolute(binary)) throw Error('Set APPARENT_CONNECT_TEST_BINARY to the rebuilt absolute apparent binary');

test('real Connect/cloud shares CLI preserves service rows, pagination and safe failures', { timeout: 120_000 }, async () => {
  const root = await realpath(await mkdtemp('/tmp/apparent-dev-shares-cli-'));
  await chmod(root, 0o700);
  const home = path.join(root, 'home');
  const state = path.join(home, '.config/apparent/hosted');
  await mkdir(state, { recursive: true, mode: 0o700 });
  const config = path.join(home, '.config/apparent/config.json');
  let mode: 'success' | 'denied' | 'wrong-url' = 'success';
  const requests: string[] = [];
  let origin = '';
  const prefix = 'opaque /?&+é';
  const cursor = prefix + '\0'.repeat(4096 - Buffer.byteLength(prefix));
  const cursorRoute = '/v1/shares?before=' + encodeURIComponent(cursor);
  const row = () => ({ id: 'a'.repeat(32), captureId: 'b'.repeat(32), status: 'ready',
    url: `${origin}/s/${'c'.repeat(64)}`, title: 'Synthetic recording',
    createdAt: '2026-11-10T00:00:00.000Z', expiresAt: '2026-11-17T00:00:00.000Z',
    extension: { retained: true } });
  const server = http.createServer((request, response) => {
    requests.push(request.url ?? '');
    if (request.headers.authorization !== 'Bearer synthetic-cli-read-token' || mode === 'denied') {
      response.writeHead(401, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ error: 'unauthorized', secret: 'synthetic-private-response' }));
      return;
    }
    response.setHeader('Content-Type', 'application/json');
    if (mode === 'wrong-url') {
      response.end(JSON.stringify({ shares: [{ ...row(), url: `https://other.example/s/${'c'.repeat(64)}` }], nextCursor: null }));
    } else if (request.url === '/v1/shares') {
      response.end(JSON.stringify({ shares: [row()], nextCursor: cursor }));
    } else if (request.url === cursorRoute) {
      response.end(JSON.stringify({ shares: [{ ...row(), id: 'd'.repeat(32), status: 'expired', url: null }], nextCursor: null }));
    } else {
      response.writeHead(404); response.end('{}');
    }
  });
  try {
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    assert.ok(address && typeof address === 'object');
    origin = `http://127.0.0.1:${address.port}`;
    const settings = JSON.stringify({ unrelated: { retained: true }, cloud: { server: origin, enabled: false, uploadConsent: false } });
    const credential = JSON.stringify({ server: origin, site: origin, scope: 'read', token: 'synthetic-cli-read-token', generation: 'e'.repeat(32) });
    await writeFile(config, settings, { mode: 0o600 });
    await writeFile(path.join(state, 'read.credential'), credential, { mode: 0o600 });
    const invoke = (name = 'connect') => new Promise<{ code: number | null; stdout: string; stderr: string }>((resolve, reject) => {
      const child = spawn(binary, [name, 'shares', 'list'], {
        env: { ...process.env, HOME: home, APPARENT_CONFIG: config }, stdio: ['ignore', 'pipe', 'pipe'],
      });
      let stdout = '', stderr = '';
      let failure: Error | undefined;
      const stop = (message: string) => { failure ??= Error(message); child.kill('SIGKILL'); };
      const timer = setTimeout(() => stop('Owned CLI exceeded 20-second fixture deadline'), 20_000);
      child.stdout.on('data', chunk => { stdout += chunk; if (stdout.length > 1_000_000) stop('Owned CLI exceeded fixture output budget'); });
      child.stderr.on('data', chunk => { stderr += chunk; if (stderr.length > 1_000_000) stop('Owned CLI exceeded fixture output budget'); });
      child.on('error', error => { clearTimeout(timer); reject(error); });
      child.on('close', code => { clearTimeout(timer); if (failure) reject(failure); else resolve({ code, stdout, stderr }); });
    });
    for (const name of ['connect', 'cloud']) {
      requests.length = 0;
      const result = await invoke(name);
      assert.equal(result.code, 0, result.stderr);
      assert.deepEqual(JSON.parse(result.stdout), { shares: [row(), { ...row(), id: 'd'.repeat(32), status: 'expired', url: null }], nextCursor: null });
      assert.deepEqual(requests, ['/v1/shares', cursorRoute]);
      assert.ok(!result.stdout.includes('synthetic-cli-read-token'));
    }
    mode = 'wrong-url';
    const malformed = await invoke();
    assert.notEqual(malformed.code, 0);
    assert.equal(malformed.stdout, '');
    assert.match(malformed.stderr, /Invalid share URL/);
    mode = 'denied';
    const denied = await invoke();
    assert.notEqual(denied.code, 0);
    assert.equal(denied.stdout, '');
    assert.match(denied.stderr, /unauthorized|auth_required/);
    assert.ok(!denied.stderr.includes('synthetic-private-response'));
    assert.ok(!denied.stderr.includes('synthetic-cli-read-token'));
    assert.equal(await readFile(config, 'utf8'), settings);
    const afterDenial = JSON.parse(await readFile(path.join(state, 'read.credential'), 'utf8'));
    assert.equal(afterDenial.token, 'synthetic-cli-read-token');
    assert.equal(afterDenial.authRequired, true);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    await rm(root, { recursive: true, force: true });
  }
});

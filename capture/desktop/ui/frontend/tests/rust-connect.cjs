// Shipped TypeScript controller in Electron Node -> production Rust listener/auth client.
// Only the HTTP device endpoint is a labelled local pending-approval fixture.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const net = require('node:net');
const path = require('node:path');
const { ConnectSettings } = require(process.env.APPARENT_CONNECT_TEST_MODULE || '../electron/connect.cjs');
const [home, config] = process.argv.slice(2);
const address = path.join(home, '.config/apparent/agent-control/capture.sock');
const state = path.join(path.dirname(config), 'hosted');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(read, accept, label) {
  const end = performance.now() + 5000;
  let last;
  while (performance.now() < end) { last = await read(); if (accept(last)) return last; await wait(30); }
  throw new Error(`${label}: ${JSON.stringify(last)}`);
}
async function cancellation() { return fs.readFile(path.join(state, 'cancellation'), 'utf8').catch(() => null); }
async function revoked(previous) {
  const raw = JSON.parse(await fs.readFile(config, 'utf8'));
  const files = await fs.readdir(state).catch(() => []);
  return (await cancellation()) !== previous && raw.cloud.enabled === false && raw.cloud.uploadConsent === false &&
    // An absent experiment is off; disable need not materialize a false key.
    raw.experiments.hostedSync !== true && !files.some(name => name.endsWith('.intent'));
}
async function raw(request, command) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(address);
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('Raw request deadline')); }, 3500);
    let bytes = '', first = true;
    socket.on('error', reject);
    socket.on('connect', () => socket.write(JSON.stringify(request) + '\n'));
    socket.on('data', chunk => {
      bytes += chunk;
      if (!bytes.endsWith('\n')) return;
      const value = JSON.parse(bytes); bytes = '';
      if (first && command && value.connect) { first = false; socket.write(JSON.stringify(command) + '\n'); return; }
      clearTimeout(timer); socket.destroy(); resolve(value);
    });
  });
}
async function main() {
  const calls = [];
  const frames = [];
  const factVersions = [];
  const createConnection = net.createConnection;
  net.createConnection = (...args) => {
    const socket = createConnection(...args);
    if (args[0] === address) {
      let received='';
      socket.on('data',chunk=>{
        received+=String(chunk);
        if(received.endsWith('\n')) {
          try {const reply=JSON.parse(received);if(reply.account?.stateFacts!==undefined)factVersions.push(reply.account.stateFacts);}catch{}
          received='';
        }
      });
      const write = socket.write.bind(socket);
      socket.write = (chunk, ...rest) => {
        try { const frame = JSON.parse(String(chunk)); frames.push({connect:frame.connect,version:frame.version,command:frame.command}); } catch {}
        return write(chunk, ...rest);
      };
    }
    return socket;
  };
  const clients = new Set();
  let client;
  let origin;
  let deviceMode = 'pending';
  const enrollment = [];
  const server = http.createServer((request, response) => {
    calls.push(request.url);
    const body = [];
    request.on('data', chunk => body.push(chunk));
    request.on('end', () => {
    response.setHeader('Content-Type', 'application/json');
    if (request.url === '/v1/device/start') enrollment.push(JSON.parse(Buffer.concat(body).toString('utf8')));
    if (deviceMode === 'offline') { response.writeHead(503); response.end('{"error":"synthetic-private-service-error"}'); return; }
    if (request.url === '/v1/device/start') return response.end(JSON.stringify({
      deviceCode: 'synthetic-private-device-secret', userCode: 'TEST-CODE', expiresIn: deviceMode === 'expired' ? 0 : 60, interval: 1,
      verificationUri: origin + '/hosted/device?code=TEST-CODE',
    }));
    if (request.url === '/v1/device/poll') return response.end(JSON.stringify(deviceMode === 'approved' ? {
      status:'approved',scope:'upload',accessToken:'synthetic-approved-access',refreshToken:'synthetic-approved-refresh',
      grantId:'synthetic-approved-grant',expiresIn:300,refreshExpiresIn:3600,accountInitial:'M',
    } : {status:deviceMode === 'rejected' ? 'rejected' : 'pending'}));
    response.writeHead(500); response.end('{"error":"unexpected_test_endpoint"}');
    });
  });
  server.on('connection', socket => { clients.add(socket); socket.on('close', () => clients.delete(socket)); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  try {
    client = new ConnectSettings(home);
    assert.equal((await client.request()).status, 'connected');
    await client.request('allow'); // Default availability never supplies consent.
    assert.equal(calls.length, 0);
    await assert.rejects(fs.stat(config), { code: 'ENOENT' });
    await fs.writeFile(config, JSON.stringify({ experiments: { connectSettings: false } }), { mode: 0o600 });
    await until(() => client.request(), r => r.status === 'disabled', 'Explicit opt-out did not hide Connect');
    assert.equal(calls.length, 0);
    const original = { experiments: { unrelated: true }, cloud: { server: origin, privateNested: { keep: true } }, privateFixture: { keep: 'synthetic' } };
    await fs.writeFile(config, JSON.stringify(original), { mode: 0o600 });
    await until(() => client.request(), r => r.status === 'connected', 'Actual Rust opening did not become ready');
    const preserved = await fs.readFile(config, 'utf8');
    await client.request();
    await client.request('allow'); // Forged allow without preceding consent is inert.
    assert.equal(calls.length, 0);
    assert.equal(await fs.readFile(config, 'utf8'), preserved);
    for (const ending of ['cancel', 'close']) {
      const consent = await client.request('connect');
      assert.ok(consent.view.buttons.some(button => button.command === 'allow'));
      const beforeNetwork = calls.length;
      await client.request('allow');
      const approval = await until(() => client.request(), r => r.view?.userCode === 'TEST-CODE', 'Actual production client public approval missing');
      assert.equal(approval.view.verificationUri, origin + '/hosted/device?code=TEST-CODE');
      assert.ok(calls.length > beforeNetwork);
      assert.equal(JSON.stringify(approval).includes('synthetic-private-device-secret'), false);
      const previous = await cancellation();
      if (ending === 'cancel') {
        await client.request('cancel');
        await until(() => client.request(), r => r.view?.buttons.some(b => b.command === 'connect' && b.enabled), 'Cancel did not return to idle');
      } else client.close();
      await until(() => revoked(previous), Boolean, `${ending} did not durably revoke`);
      await wait(1200); // Cross one actual production device-poll/cancellation interval.
      const files = await fs.readdir(state);
      assert.equal(files.some(name => name.endsWith('.credential')), false);
      const saved = JSON.parse(await fs.readFile(config, 'utf8'));
      assert.deepEqual(saved.privateFixture, original.privateFixture);
      assert.deepEqual(saved.cloud.privateNested, original.cloud.privateNested);
      if (ending === 'close') await until(() => client.request(), r => r.status === 'connected', 'Closed stream did not reopen locally');
    }
    for (const expected of ['rejected', 'expired', 'offline']) {
      deviceMode = expected;
      await client.request('connect');
      await client.request('allow');
      const failed = await until(() => client.request(), r => r.view?.presentationState?.error === expected,
        `Actual account ${expected} result was not projected`);
      assert.equal(JSON.stringify(failed).includes('synthetic-private'), false);
      assert.ok(failed.view.buttons.some(button => button.command === 'connect' && button.enabled));
    }
    deviceMode = 'approved';
    await client.request('connect'); await client.request('allow');
    const committed = await until(() => client.request(), r => r.view?.presentationState?.snapshot?.uploadsEnabled === true &&
      r.view.presentationState.operation === null, 'TypeScript device approval did not commit/activate through the real writer');
    assert.equal(JSON.stringify(committed).includes('synthetic-approved'), false);
    const durableGrant = JSON.parse(await fs.readFile(path.join(state,'upload.credential'),'utf8'));
    assert.equal(durableGrant.token,'synthetic-approved-access'); assert.equal(durableGrant.refreshToken,'synthetic-approved-refresh');
    assert.equal(durableGrant.server,origin); assert.equal(durableGrant.grantId,'synthetic-approved-grant');
    assert.match(durableGrant.encryption_identity,/^[a-f0-9]{32}$/);
    await client.request('request-disable');
    assert.equal(JSON.parse(await fs.readFile(config,'utf8')).cloud.enabled,true,'Confirmation must not revoke before explicit action');
    await client.request('disable');
    await until(()=>client.request(),r=>r.view?.presentationState?.snapshot?.uploadsEnabled === false && r.view.presentationState.operation === null,'Approved grant disable did not settle');
    assert.ok(factVersions.includes(1),'Real runtime must exercise the TypeScript facts projection');
    assert.ok(frames.some(frame=>frame.command === 'device-login'));
    assert.ok(frames.some(frame=>frame.command === 'device-poll'));
    assert.ok(frames.some(frame=>frame.command === 'device-commit'));
    assert.equal(frames.some(frame=>frame.command === 'login'),false,'Shipped device protocol must execute in TypeScript');
    deviceMode = 'pending';
    assert.ok(frames.some(frame => frame.connect === 'backend' && frame.version === 1));
    assert.equal(frames.some(frame => frame.connect === 'settings'), false, 'New runtime must exercise the TypeScript controller, not the legacy fallback');
    assert.ok(enrollment.length >= 5);
    for (const request of enrollment) {
      assert.equal(request.scope, 'upload');
      assert.deepEqual(Object.keys(request).sort(), ['encryptionKey', 'scope']);
      assert.equal(request.encryptionKey.version, 1);
      assert.match(request.encryptionKey.recipient, /^age1[a-z0-9]{58}$/);
      assert.match(request.encryptionKey.signing, /^[a-f0-9]{64}$/);
    }
    client.close(); await wait(80);
    assert.equal((await raw({ connect: 'settings', deadline: 1 })).error, 'admission_expired');
    assert.equal((await raw({ connect: 'settings', deadline: Number(process.hrtime.bigint()/1000000n)+3000, secret: 'synthetic' })).error, 'invalid_request');
    const count = calls.length;
    const expired = await raw({ connect: 'settings', deadline: Number(process.hrtime.bigint()/1000000n)+3000 },
      { connect: 'command', command: 'allow', deadline: 1 });
    assert.equal(expired.error, 'admission_expired');
    assert.equal(calls.length, count);
    assert.ok(calls.every(url => ['/v1/device/start', '/v1/device/poll'].includes(url)));
    // Synthetic local grants exercise the actual Rust projection, not account approval.
    const grant = {server:origin,scope:'upload',token:'synthetic-profile-token',generation:'a'.repeat(32),accountInitial:'M'};
    const credential = path.join(state, 'upload.credential');
    await fs.writeFile(credential, JSON.stringify(grant), {mode:0o600});
    await fs.writeFile(config, JSON.stringify({...original,cloud:{...original.cloud,enabled:true,uploadConsent:true}}), {mode:0o600});
    const identity = await client.request();
    assert.equal(identity.view.presentationState.snapshot.accountInitial, 'M');
    assert.equal(JSON.stringify(identity).includes(grant.token), false);
    await fs.writeFile(credential, JSON.stringify({...grant,authRequired:true}), {mode:0o600});
    assert.equal((await client.request()).view.presentationState.snapshot.accountInitial, null);
    const expiredAt=Math.floor(Date.now()/1000)-1;
    await fs.writeFile(credential,JSON.stringify({...grant,expiresAt:expiredAt}),{mode:0o600});
    assert.equal((await client.request()).view.presentationState.snapshot.reauthorizationRequired,true);
    await fs.writeFile(credential,JSON.stringify({...grant,expiresAt:expiredAt,refreshToken:'synthetic-renewable',
      refreshExpiresAt:expiredAt+3600,grantId:'synthetic-renewable-grant'}),{mode:0o600});
    const renewable=(await client.request()).view.presentationState.snapshot;
    assert.equal(renewable.connected,false);assert.equal(renewable.renewalRequired,true);assert.equal(renewable.accountInitial,'M');
    await fs.unlink(credential);
    assert.equal((await client.request()).view.presentationState.snapshot.accountInitial, null);
    assert.equal(calls.length, count, 'Identity display must stay local-only');
    console.log(JSON.stringify({ platform: process.platform, electron: process.versions.electron,
      bridge: 'shipped TypeScript controller + real Rust listener/auth client', controllerProtocol:'backend-v1',snapshotPolicy:'typescript-facts-v1',
      packagedController:process.env.APPARENT_CONNECT_TEST_MODULE !== undefined,
      http: 'synthetic loopback device approval; private isolated credentials, no uploads',
      checks: 'disabled/local-only/no-consent/public-projection/cancel/EOF/durable-off/preserved-config/approval-commit/rejection/expiry/offline/retry/key-enrollment/expired-frame/account-initial/revoked-identity', requests: calls.length }));
  } catch (error) {
    console.error(JSON.stringify({ fixtureEndpointsObserved: calls }));
    throw error;
  } finally {
    net.createConnection = createConnection;
    client?.close(); for (const socket of clients) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  }
}
const watchdog = setTimeout(() => { console.error('Actual Rust Connect integration deadline'); process.exit(1); }, 20000);
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => clearTimeout(watchdog));

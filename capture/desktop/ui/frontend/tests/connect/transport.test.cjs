const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const { ConnectSettings, browserUri } = require('../../electron/connect.cjs');
const view = { title: 'Apparent Connect', status: 'Not connected', detail: 'Local only', note: 'No public links',
  server: null, verificationUri: null, userCode: null, buttons: [{ command: 'connect', title: 'Sign in', enabled: true }] };
const semantic = { snapshot:{connected:false,uploadsEnabled:false,renewalRequired:false,reauthorizationRequired:false,accountInitial:null},
  health:null,error:null,localError:null,consent:false,approval:false,operation:null,logout:false,
  retryDisable:null,confirmation:null,closed:false,sharedLinksUrl:null };
async function fixture(t, respond = () => ({ connect: view })) {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'co-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  await fs.chmod(directory, 0o700);
  const clients = new Set(), requests = [];
  let closed = 0;
  const server = net.createServer(socket => {
    clients.add(socket);
    socket.on('error', () => {});
    socket.on('close', () => { clients.delete(socket); closed += 1; });
    let bytes = '';
    socket.on('data', chunk => {
      bytes += chunk;
      while (bytes.includes('\n')) {
        const end = bytes.indexOf('\n');
        const request = JSON.parse(bytes.slice(0, end)); bytes = bytes.slice(end + 1);
        requests.push(request);
        const result = respond(request, socket);
        if (result !== undefined) socket.write(JSON.stringify(result) + '\n');
      }
    });
  });
  const address = path.join(directory, 'capture.sock');
  await new Promise(resolve => server.listen(address, resolve));
  await fs.chmod(address, 0o600);
  const client = new ConnectSettings(home);
  t.after(async () => {
    client.close();
    for (const socket of clients) socket.destroy();
    await new Promise(resolve => server.close(resolve));
    await fs.rm(home, { recursive: true, force: true });
  });
  return { client, home, address, directory, requests, clients, closed: () => closed };
}
test('semantic facts strip unknown secrets and cannot authorize unrelated browser actions', async t => {
  const f = await fixture(t, () => ({connect:{...view,presentationState:{...semantic,token:'secret',
    snapshot:{...semantic.snapshot,token:'nested-secret'}}}}));
  assert.deepEqual((await f.client.request()).view.presentationState, semantic);
});
test('an explicit semantic shared-link denial cannot fall back to the legacy URL', async t => {
  const url = 'https://connect.test/connect/shares';
  const presentation = {tone:'neutral',headline:'Connect',detail:'Local',note:'',artwork:false,compact:false,accountMenu:true,
    buttons:[{command:'open-shared-links',title:'Manage',enabled:true}],menuActions:['open-shared-links'],sharedLinksUrl:url};
  const f = await fixture(t, request => ({connect:{...view,presentation,presentationState:semantic},
    ...(request.command === 'open-shared-links' ? {openUri:url} : {})}));
  assert.equal((await f.client.request()).status, 'connected');
  assert.deepEqual(await f.client.request('open-shared-links'), {status:'error'});
});
for (const patch of [{operation:'upload'}, {error:'secret diagnostic'}, {snapshot:{...semantic.snapshot,accountInitial:'private@example.com'}},
  {confirmation:'yes'}, {sharedLinksUrl:'https://connect.test/other'}]) {
  test('malformed semantic facts close without falling back to old presentation '+JSON.stringify(patch), async t => {
    const f = await fixture(t, () => ({connect:{...view,presentationState:{...semantic,...patch}}}));
    assert.deepEqual(await f.client.request(), {status:'error'});
    assert.equal(f.client.socket, undefined);
  });
}
test('Connect stream opens locally, reuses one controller, strips unknown fields and never supplies server/URI', async t => {
  const f = await fixture(t, () => ({ connect: { ...view, token: 'synthetic-secret' }, config: 'synthetic-secret' }));
  assert.deepEqual(await f.client.request(), { status: 'connected', view });
  assert.deepEqual(await f.client.request('connect'), { status: 'connected', view });
  assert.deepEqual(await f.client.request(), { status: 'connected', view });
  assert.equal(f.clients.size, 1);
  assert.deepEqual(f.requests.map(r => r.connect), ['settings', 'command', 'read']);
  assert.equal(f.requests[1].command, 'connect');
  assert.deepEqual(Object.keys(f.requests[1]).sort(), ['command', 'connect', 'deadline']);
  assert.ok(f.requests.every(r => Number.isSafeInteger(r.deadline)));
  await assert.rejects(f.client.request('arbitrary'), /Invalid Connect action/);
});
test('Connect mutations require an observed controller and unsafe socket refuses all contact', async t => {
  const f = await fixture(t);
  assert.deepEqual(await f.client.request('allow'), { status: 'error' });
  assert.equal(f.requests.length, 0);
  await fs.chmod(f.address, 0o666);
  assert.deepEqual(await f.client.request(), { status: 'error' });
  assert.equal(f.requests.length, 0);
  await fs.chmod(f.address, 0o600);
  await fs.chmod(f.directory, 0o755);
  assert.deepEqual(await f.client.request(), { status: 'error' });
  assert.equal(f.requests.length, 0);
});
test('Connect close invalidates in-flight receipt, closes controller and reopen uses a fresh stream', async t => {
  let hold;
  const f = await fixture(t, (request, socket) => {
    if (request.command === 'allow') { hold = () => socket.write(JSON.stringify({ connect: view }) + '\n'); return; }
    return { connect: view };
  });
  await f.client.request();
  const pending = f.client.request('allow');
  while (!hold) await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(await f.client.request('cancel'), { status: 'error' });
  f.client.close();
  assert.deepEqual(await pending, { status: 'cancelled' });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(f.closed(), 1);
  assert.deepEqual(await f.client.request(), { status: 'connected', view });
  assert.equal(f.requests.at(-1).connect, 'settings');
});
test('Connect timeout closes ambiguous controller without retrying a command', { timeout: 2000 }, async t => {
  let received;
  const admitted = new Promise(resolve => { received = resolve; });
  const f = await fixture(t, request => {
    if (request.command) { received(); return; }
    return { connect: view };
  });
  assert.deepEqual(await f.client.request(), { status: 'connected', view });
  f.client.timeout = 150;
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let settled = false;
  const pending = f.client.request('allow').then(result => { settled = true; return result; });
  await admitted;
  t.mock.timers.tick(149);
  await new Promise(setImmediate);
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  assert.deepEqual(await pending, { status: 'cancelled' });
  assert.equal(f.client.socket, undefined);
  assert.equal(f.requests.filter(r => r.command === 'allow').length, 1);
});

for (const response of [{ error: 'feature_disabled' }, { error: 'synthetic-secret' },
  { connect: { ...view, userCode: 'secret with spaces' } },
  { connect: view, openUri: 'https://example.com/' }]) {
  test(`Connect fixed errors/invalid public response close the stream (${JSON.stringify(response).slice(0, 50)})`, async t => {
    const f = await fixture(t, () => response);
    assert.deepEqual(await f.client.request(), { status: response.error === 'feature_disabled' ? 'disabled' : 'error' });
    assert.equal(f.client.socket, undefined);
  });
}
test('Connect browser URI is returned only for explicit current action and validated public code', async t => {
  const approval = { ...view, verificationUri: 'http://127.0.0.1:38761/hosted/device?code=TEST-CODE', userCode: 'TEST-CODE' };
  const f = await fixture(t, request => ({ connect: approval,
    ...(request.command === 'open-browser' ? { openUri: approval.verificationUri } : {}) }));
  const initial = await f.client.request();
  assert.equal(initial.openUri, undefined);
  assert.equal((await f.client.request('open-browser')).openUri, approval.verificationUri);
  for (const bad of ['file:///etc/passwd', 'javascript:alert(1)', 'https://user:secret@example.com/',
    'http://example.com/', 'https://example.com/?deviceCode=secret', 'https://example.com/#secret',
    'https://example.com/%20', 'https://example.com/?code=OTHER']) assert.equal(browserUri(bad, 'TEST-CODE'), false);
});
test('Connect oversized/compound frame closes before exposing a response', async t => {
  const f = await fixture(t, (_request, socket) => { socket.write('x'.repeat(65537)); });
  assert.deepEqual(await f.client.request(), { status: 'error' });
  assert.equal(f.client.socket, undefined);
});

test('typed presentation is bounded, strips unknown data and admits only the explicit shared-links URI action', async t => {
  const presentation = { tone:'success', headline:'Connected', detail:'Private uploads', note:'', artwork:true,
    compact:true, accountMenu:true, accountInitial:'M', buttons:[{command:'open-shared-links',title:'Manage shared links',enabled:true}],
    menuActions:['open-shared-links'], sharedLinksUrl:'https://example.com/hosted/shares' };
  const f = await fixture(t, request => ({ connect:{...view,presentation:{...presentation,secret:'hidden'}},
    ...(request.command === 'open-shared-links' ? { openUri:presentation.sharedLinksUrl } : {}) }));
  assert.deepEqual((await f.client.request()).view.presentation, presentation);
  assert.equal((await f.client.request('open-shared-links')).openUri, presentation.sharedLinksUrl);
});
for (const invalid of [{tone:'purple'}, {menuActions:['disconnect']}, {sharedLinksUrl:'https://example.com/unrelated'}, {accountInitial:'m@example.com'}, {accountInitial:'\u202eM'}, {accountInitial:'ABCD'}]) {
  test('rejects malformed semantic presentation '+JSON.stringify(invalid), async t => {
    const presentation = {tone:'neutral',headline:'Connect',detail:'Local',note:'',artwork:false,compact:false,accountMenu:false,
      buttons:[],menuActions:[],sharedLinksUrl:null,...invalid};
    const f = await fixture(t, () => ({connect:{...view,presentation}}));
    assert.equal((await f.client.request()).status,'error');
  });
}

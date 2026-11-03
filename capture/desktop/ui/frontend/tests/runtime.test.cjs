const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const { readRuntimeState, captureLocation, errorReporting, captureReference, storagePreference } = require('../electron/runtime.cjs');

async function fixture(t, reply, { mode = 0o700 } = {}) {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode });
  await fs.chmod(directory, mode);
  const requests = [];
  const clients = new Set();
  const server = net.createServer((socket) => {
    clients.add(socket);
    socket.on('close', () => clients.delete(socket));
    socket.on('error', () => {});
    socket.once('data', (request) => {
      requests.push(request.toString());
      if (typeof reply === 'function') reply(socket,request);
      else if (reply !== null) socket.end(reply);
    });
  });
  const address = path.join(directory, 'capture.sock');
  await new Promise((resolve) => server.listen(address, resolve));
  await fs.chmod(address, 0o600);
  t.after(async () => {
    for (const client of clients) client.destroy();
    await new Promise((resolve) => server.close(resolve));
    await fs.rm(home, { recursive: true, force: true });
  });
  return { home, directory, address, requests };
}

test('reads only state and strips capture identity and unexpected data', async (t) => {
  const f = await fixture(t, '{"state":{"phase":"recording","capture_id":"private-id"},"secret":"hidden"}\n');
  assert.deepEqual(await readRuntimeState(f.home), { status: 'connected', phase: 'recording' });
  assert.deepEqual(f.requests, ['state\n']);
});

test('missing GUI is disconnected and creates no config directory', async (t) => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  t.after(() => fs.rm(home, { recursive: true, force: true }));
  assert.deepEqual(await readRuntimeState(home), { status: 'disconnected' });
  await assert.rejects(fs.stat(path.join(home, '.config')), { code: 'ENOENT' });
});

test('rejects public directories before contacting the socket', async (t) => {
  const f = await fixture(t, '{}\n', { mode: 0o755 });
  assert.equal((await readRuntimeState(f.home)).status, 'error');
  assert.deepEqual(f.requests, []);
});

test('rejects symlinked sockets and unsafe socket modes without a request', async (t) => {
  const f = await fixture(t, '{}\n');
  await fs.chmod(f.address, 0o666);
  assert.equal((await readRuntimeState(f.home)).status, 'error');
  await fs.rename(f.address, f.address + '.real');
  await fs.symlink(f.address + '.real', f.address);
  assert.equal((await readRuntimeState(f.home)).status, 'error');
  assert.deepEqual(f.requests, []);
});

for (const reply of ['not json\n', '{}\n', '{"state":{"phase":"new_phase"}}\n', '{"error":"app_busy"}\n', 'x'.repeat(4097), '{"state":{"phase":"idle"}}']) {
  test(`invalid, incomplete or oversized reply fails closed (${reply.slice(0, 22)})`, async (t) => {
    const f = await fixture(t, reply);
    assert.equal((await readRuntimeState(f.home)).status, 'error');
  });
}

test('absolute deadline bounds an unresponsive server', async (t) => {
  const f = await fixture(t, null);
  const before = performance.now();
  assert.equal((await readRuntimeState(f.home, 50)).status, 'error');
  assert.ok(performance.now() - before < 1000);
  assert.deepEqual(f.requests, ['state\n']);
});

test('capture reference is a bounded ID-only read and exact conditional copy', async (t) => {
  const id = 'older capture — selected.apparent';
  const f = await fixture(t, JSON.stringify({ reference: { id, copied: false, path: '/private', token: 'hidden' } }) + '\n');
  assert.deepEqual(await captureReference(f.home), { status: 'connected', id, copied: false });
  const read = JSON.parse(f.requests[0]);
  assert.deepEqual(Object.keys(read).sort(), ['deadline', 'reference']);
  const copied = await fixture(t, JSON.stringify({ reference: { id, copied: true } }) + '\n');
  assert.deepEqual(await captureReference(copied.home, 'copy', id), { status: 'connected', id, copied: true });
  assert.equal(JSON.parse(copied.requests[0]).expected, id);
  assert.throws(() => captureReference(f.home, 'copy', '/private/path'));
  assert.throws(() => captureReference(f.home, 'copy', ''));
  assert.throws(() => captureReference(f.home, 'upload', id));
});

for (const [reply, expected] of [
  [{ reference: { id: null, copied: false } }, { status: 'connected', id: null, copied: false }],
  [{ error: 'feature_disabled' }, { status: 'disabled' }],
  [{ error: 'invalid_request' }, { status: 'incompatible' }],
  [{ error: 'conflict', privatePath: '/private' }, { status: 'error', code: 'conflict' }],
  [{ error: 'private backend detail' }, { status: 'error', code: 'invalid_response' }],
  [{ reference: { id: '/private/path', copied: false } }, { status: 'error', code: 'invalid_response' }],
  [{ reference: { id: 'capture.apparent', copied: true } }, { status: 'error', code: 'invalid_response' }],
]) test(`capture reference projects only allowed outcomes ${JSON.stringify(reply)}`, async (t) => {
  const f = await fixture(t, JSON.stringify(reply) + '\n');
  assert.deepEqual(await captureReference(f.home), expected);
});

test('copy mismatch and timeout never become success or automatic retries', async (t) => {
  const wrong = await fixture(t, '{"reference":{"id":"other.apparent","copied":true}}\n');
  assert.deepEqual(await captureReference(wrong.home, 'copy', 'selected.apparent'), { status: 'error', code: 'invalid_response' });
  const stalled = await fixture(t, null);
  assert.deepEqual(await captureReference(stalled.home, 'copy', 'selected.apparent', 50), { status: 'error', code: 'outcome_unknown' });
  assert.equal(stalled.requests.length, 1);
});

const snapshot = { savedPath: '/saved', effectivePath: '/override', displayPath: '/override',
  custom: true, source: 'environment', revision: 'a'.repeat(16) };
test('retention and usage are explicitly projected; malformed policy fails closed', async t => {
  const r = { revision: 'a'.repeat(16), custom: false, retention: { preset: 'week', note: 'Rust note',
    options: ['week', 'month', 'forever'].map(id => ({ id, label: id, secret: 'hidden' })) }, secret: 'hidden' };
  const f = await fixture(t, JSON.stringify(r) + '\n');
  const decoded = await storagePreference(f.home, 'retention');
  assert.equal(decoded.secret, undefined); assert.equal(decoded.retention.options[0].secret, undefined);
  await storagePreference(f.home, 'saveretention', { expected: r.revision, preset: 'month', secret: 'hidden' });
  assert.deepEqual({ ...JSON.parse(f.requests[1]), deadline: 0 }, { storage: 'saveretention', expected: r.revision, preset: 'month', deadline: 0 });
  assert.throws(() => storagePreference(f.home, 'saveretention', { expected: r.revision, preset: '/arbitrary' }));
  const invalid = await fixture(t, JSON.stringify({ ...r, retention: { ...r.retention, options: [] } }) + '\n');
  assert.deepEqual(await storagePreference(invalid.home, 'retention'), { status: 'error', code: 'invalid_response' });
  const usage = await fixture(t, JSON.stringify({ usage: { effectivePath: '/captures', displayPath: '~/captures', formatted: '1 MB', secret: 'hidden' } }) + '\n');
  assert.deepEqual(await storagePreference(usage.home, 'usage'), { status: 'connected', usage: { effectivePath: '/captures', displayPath: '~/captures', formatted: '1 MB' } });
  const stalled = await fixture(t, null);
  assert.deepEqual(await storagePreference(stalled.home, 'saveretention', { expected: r.revision, preset: 'month' }, 50), { status: 'error', code: 'outcome_unknown' });
  assert.equal(stalled.requests.length, 1);
});
test('storage uses typed requests and projects only permitted snapshot fields', async (t) => {
  const f = await fixture(t, JSON.stringify({ storage: { ...snapshot, token: 'private' }, secret: 'hidden' }) + '\n');
  assert.deepEqual(await captureLocation(f.home), { status: 'connected', storage: snapshot });
  const request = JSON.parse(f.requests[0]);
  assert.deepEqual(Object.keys(request).sort(), ['deadline', 'storage']);
  assert.equal(request.storage, 'read');
  assert.ok(request.deadline > Number(process.hrtime.bigint() / 1000000n));
  await captureLocation(f.home, 'set', { path: '/space é/folder', expected: snapshot.revision, secret: 'ignored' });
  const set = JSON.parse(f.requests[1]);
  assert.deepEqual({ ...set, deadline: 0 }, { storage: 'set', path: '/space é/folder', expected: snapshot.revision, deadline: 0 });
  await captureLocation(f.home, 'reset', { expected: snapshot.revision });
  assert.equal(JSON.parse(f.requests[2]).storage, 'reset');
});
test('error reporting bridge projects only boolean state and accepts only typed toggles', async (t) => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'ui-'));
  const directory = path.join(home, '.config/apparent/agent-control');
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const requests = [];
  let enabled = true;
  const server = net.createServer((socket) => socket.once('data', (bytes) => {
    const request = JSON.parse(bytes.toString()); requests.push(request);
    if (request.errorReporting === 'set') enabled = request.enabled;
    socket.end(JSON.stringify({ errorReporting: { enabled, secret: 'private' }, secret: 'private' }) + '\n');
  }));
  const address = path.join(directory, 'capture.sock');
  await new Promise((resolve) => server.listen(address, resolve)); await fs.chmod(address, 0o600);
  t.after(async () => { await new Promise((resolve) => server.close(resolve)); await fs.rm(home, { recursive: true, force: true }); });
  assert.deepEqual(await errorReporting(home), { status: 'connected', enabled: true });
  assert.deepEqual(await errorReporting(home, false), { status: 'connected', enabled: false });
  assert.deepEqual(await errorReporting(home), { status: 'connected', enabled: false });
  assert.deepEqual(requests.map(({ errorReporting: op, enabled: value }) => [op, value]), [['read', undefined], ['set', false], ['read', undefined]]);
  assert.throws(() => errorReporting(home, 'false'), /Invalid reporting preference/);
});

test('storage refuses unsafe permissions and times out mutations without retry', async (t) => {
  const unsafe = await fixture(t, '{}\n', { mode: 0o755 });
  assert.equal((await captureLocation(unsafe.home)).status, 'error');
  assert.deepEqual(unsafe.requests, []);
  const stalled = await fixture(t, null);
  assert.deepEqual(await captureLocation(stalled.home, 'reset', { expected: snapshot.revision }, 50),
    { status: 'error', code: 'outcome_unknown' });
  assert.equal(stalled.requests.length, 1);
});
for (const [reply, expected] of [
  [{ error: 'invalid_request', secret: 'hidden' }, { status: 'incompatible' }],
  [{ error: 'feature_disabled' }, { status: 'disabled' }],
  [{ error: 'conflict', message: 'secret' }, { status: 'error', code: 'conflict' }],
  [{ error: 'raw config secret' }, { status: 'error', code: 'invalid_response' }],
  [{ storage: { ...snapshot, custom: false } }, { status: 'error', code: 'invalid_response' }],
  [{ storage: { ...snapshot, effectivePath: 123 } }, { status: 'error', code: 'invalid_response' }],
  [{ storage: { ...snapshot, revision: 'secret' } }, { status: 'error', code: 'invalid_response' }],
]) test(`storage validates response ${JSON.stringify(reply).slice(0, 60)}`, async (t) => {
  const f = await fixture(t, JSON.stringify(reply) + '\n');
  assert.deepEqual(await captureLocation(f.home), expected);
});

test('drawing undo binds recording identity and strips unexpected state', async (t) => {
  const { drawing } = require('../electron/runtime.cjs');
  const f = await fixture(t, '{"drawing":{"revision":"0123456789abcdef","strokes":2,"private":"hidden"}}\n');
  assert.deepEqual(await drawing(f.home, 'undo', '0123456789abcdef'), { status: 'connected', revision: '0123456789abcdef', strokes: 2 });
  const request = JSON.parse(f.requests[0]);
  assert.equal(request.drawing, 'undo');
  assert.equal(request.expected, '0123456789abcdef');
  assert.ok(request.deadline > 0);
  await assert.rejects(async () => drawing(f.home, 'undo', 'wrong'));
  const stale = await fixture(t, '{"error":"conflict"}\n');
  assert.deepEqual(await drawing(stale.home, 'undo', '0123456789abcdef'), { status: 'error', code: 'conflict' });
});

const {PublicShare} = require('../electron/public-share.cjs');
test('public share requires observed selection, retains operation in main and delivers once',async t=>{
 const operation='a'.repeat(32);
 const f=await fixture(t,(socket,raw)=>{
  const r=JSON.parse(raw);assert.ok(Number.isSafeInteger(r.deadline));
  socket.end(JSON.stringify({publicShare:r.publicShare==='start'?{operation,state:'preparing'}:{state:r.publicShare==='read'?'prepared':r.publicShare==='deliver'?'copied':'cancelled'}})+'\n');
 });
 const owner=new PublicShare(f.home);
 assert.equal((await owner.request('start',null)).code,'refresh_required');
 assert.equal(f.requests.length,0);
 assert.deepEqual(await owner.request('start','selected.apparent'),{status:'connected',state:'preparing'});
 assert.deepEqual(await owner.request('read'),{status:'connected',state:'copied'});
 assert.equal((await owner.request('read')).code,'no_operation');
 const requests=f.requests.map(r=>JSON.parse(r));
 assert.equal(requests[0].expected,'selected.apparent');assert.equal(requests[0].disclosed,true);
 assert.equal(requests.filter(r=>r.publicShare==='deliver').length,1);
 assert.equal(requests[1].operation,operation);
});
test('closing public-share window rejects late start receipt and never delivers',async t=>{
 let started;
 const arrived=new Promise(r=>started=r);
 let release;
 const f=await fixture(t,(socket,raw)=>{
  if(JSON.parse(raw).publicShare==='start'){release=()=>socket.end(JSON.stringify({publicShare:{operation:'b'.repeat(32),state:'preparing'}})+'\n');started();}
  else socket.end('{"publicShare":{"state":"cancelled"}}\n');
 });
 const owner=new PublicShare(f.home), pending=owner.request('start','selected.apparent');
 await arrived;owner.close();release();
 assert.deepEqual(await pending,{status:'cancelled'});
 assert.ok(!f.requests.some(r=>JSON.parse(r).publicShare==='deliver'));
});

{
const {test}=require('node:test');const assert=require('node:assert/strict');const {parseAction,lines}=require('../electron/onboarding-protocol.cjs');
test('bounded action contract',()=>{assert.deepEqual(parseAction('use-current-shortcut'),{action:'use-current-shortcut'});assert.deepEqual(parseAction('grant:microphone'),{action:'grant',permission:'microphone'});for(const value of ['start-capture','grant:other',null,{},['continue']])assert.throws(()=>parseAction(value));const value={action:'shortcut',kind:'record',keyCode:15,modifiers:6144,key:'r',reserved:false};assert.deepEqual(parseAction(value),value);for(const changed of [{extra:true},{keyCode:'15'},{reserved:true},{modifiers:1},{kind:'other'}])assert.throws(()=>parseAction({...value,...changed}))});
test('stdin accepts fragmented lines and rejects oversized individual lines',()=>{const received=[];const feed=lines(v=>received.push(v));feed('{"type":');feed('"state"}\ninvalid\n{"type":"show"}\n');assert.equal(received.length,2);assert.throws(()=>feed('x'.repeat(65537)));const many=lines(()=>{});many('{}\n'.repeat(30000))});

}

{
  const vm = require('node:vm');
  const fs = require('node:fs');
  test('onboarding replays parent messages received before React subscribes', () => {
    let receive;
    let bridge;
    const electron = {
      ipcRenderer: { on: (_channel, listener) => { receive = listener; }, invoke: () => Promise.resolve() },
      contextBridge: { exposeInMainWorld: (_name, value) => { bridge = value; } },
    };
    vm.runInNewContext(fs.readFileSync(require.resolve('../electron/onboarding-preload.cjs'), 'utf8'), {
      require: name => { assert.equal(name, 'electron'); return electron; },
    });
    receive({}, { type: 'shortcut', label: 'Alt+Shift+S' });
    receive({}, { type: 'state', step: 'try-it' });
    const messages = [];
    const unsubscribe = bridge.subscribe(value => messages.push(value));
    assert.equal(messages[0].label, 'Alt+Shift+S');
    assert.equal(messages[1].step, 'try-it');
    receive({}, { type: 'shortcut', label: 'Ctrl+Alt+K' });
    assert.equal(messages.at(-1).label, 'Ctrl+Alt+K');
    unsubscribe();
    receive({}, { type: 'state', step: 'done' });
    assert.equal(messages.length, 3);
    const replay = [];
    bridge.subscribe(value => replay.push(value));
    assert.equal(replay.length, 2);
    assert.equal(replay[0].label, 'Ctrl+Alt+K');
    assert.equal(replay[1].step, 'done');
  });
}

{
  const {EventEmitter} = require('node:events');
  const {parentWriter} = require('../electron/onboarding-protocol.cjs');
  test('onboarding closes once when its parent pipe fails during shutdown', () => {
    const stream = new EventEmitter();
    let exits = 0;
    let writes = 0;
    stream.write = () => { writes += 1; stream.emit('error', Object.assign(new Error('closed parent'), {code:'EPIPE'})); };
    const send = parentWriter(stream, () => { exits += 1; });
    assert.doesNotThrow(() => send({action:'close'}));
    send({action:'close'});
    stream.emit('error', new Error('late stream error'));
    assert.equal(exits, 1);
    assert.equal(writes, 1);
  });
}

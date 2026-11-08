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

test('absolute deadline bounds an unresponsive server', { timeout: 1000 }, async (t) => {
  let received;
  const request = new Promise((resolve) => { received = resolve; });
  let closed;
  const connectionClosed = new Promise((resolve) => { closed = resolve; });
  const f = await fixture(t, (socket) => {
    socket.once('close', () => closed());
    received();
  });
  // Keep real socket I/O, but advance the deadline only after its request arrived.
  // A loaded runner may otherwise spend the entire 50 ms on filesystem admission.
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let settled = false;
  const result = readRuntimeState(f.home, 50).then((value) => { settled = true; return value; });
  await request;
  t.mock.timers.tick(49);
  await new Promise(setImmediate);
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  assert.equal((await result).status, 'error');
  await connectionClosed;
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
  test('onboarding replays parent messages received before React subscribes', async () => {
    let bridge, receive;
    let current={step:'try-it',shortcut:'Alt+Shift+S'};
    const electron={ipcRenderer:{on:(_channel,listener)=>{receive=listener},removeListener:()=>{receive=null},invoke:()=>Promise.resolve(current)},contextBridge:{exposeInMainWorld:(name,value)=>{if(name==='nativeOnboarding')bridge=value}}};
    vm.runInNewContext(fs.readFileSync(require.resolve('../electron/onboarding-preload.cjs'),'utf8'),{require:()=>electron});
    const messages=[];const off=bridge.onState(value=>messages.push(value));
    // The current renderer subscribes before requesting the authoritative snapshot.
    assert.equal((await bridge.readState()).shortcut,'Alt+Shift+S');
    current={step:'shortcut',shortcut:'Ctrl+Alt+K'};receive({},current);
    assert.equal(messages.at(-1).shortcut,'Ctrl+Alt+K');off();assert.equal(receive,null);
    current={step:'done',shortcut:'Ctrl+Alt+K'};
    bridge.onState(value=>messages.push(value));
    assert.equal((await bridge.readState()).step,'done');
    assert.equal(messages.length,1);
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

test('onboarding coach avoids its board and pill on offset displays, preserves identity', () => {
  const { coachPosition, project } = require('../electron/onboarding.cjs');
  for (const area of [{x:0,y:25,width:1440,height:875},{x:-1920,y:40,width:1920,height:1040}]) {
    const board=[area.x+260,area.y+160,760,590], anchor=[area.x+500,area.y+15,320,50];
    const p=coachPosition(area,anchor,board,'board-ink');
    assert.ok(p.x>=area.x && p.y>=area.y && p.x+380<=area.x+area.width && p.y+440<=area.y+area.height);
    for(const [x,y,w,h] of [board,anchor]) assert.ok(p.x+380<=x || p.x>=x+w || p.y+440<=y || p.y>=y+h);
    assert.equal(p.pointsUp,false);
  }
  const state={type:'state',step:'try-it',rows:[],ready:true,testInstance:true,practiceBoard:[0,0,700,600]};
  assert.equal(project(state).testInstance,true);
  assert.deepEqual(project(state).practiceBoard,[0,0,700,600]);
  assert.equal(project({...state,practiceBoard:[0,0,-1,600]}).practiceBoard,undefined);
});

test('onboarding output failure retires the channel once and preserves its error', async () => {
  const { Writable }=require('node:stream');
  const { parentChannel }=require('../electron/onboarding.cjs');
  for(const code of ['EPIPE','EIO']){
    const error=Object.assign(new Error(code),{code}),failures=[];
    const output=new Writable({write(_bytes,_encoding,callback){callback(error)}});
    const channel=parentChannel(output,failure=>failures.push(failure));
    channel.send('continue');
    await new Promise(resolve=>setImmediate(resolve));
    assert.deepEqual(failures,[error]);
    assert.equal(channel.send('close'),false);
  }
});

test('permission refresh keeps setup unfocused and avoids repeating native geometry calls',()=>{
 const vm=require('node:vm'),fs=require('node:fs'),{EventEmitter}=require('node:events');
 const stdin=new EventEmitter();stdin.setEncoding=stdin.resume=()=>{};
 const stdout=new EventEmitter();stdout.write=()=>true;
 const calls=[];let window;
 class Window extends EventEmitter {
  constructor(){super();window=this;this.visible=false;this.webContents=new EventEmitter();this.webContents.setWindowOpenHandler=()=>{};this.webContents.send=()=>calls.push('state');}
  removeMenu(){} loadURL(){} isDestroyed(){return false} isVisible(){return this.visible}
  show(){this.visible=true;calls.push('show')} showInactive(){this.visible=true;calls.push('showInactive')} hide(){this.visible=false}
  setAlwaysOnTop(){calls.push('top')} setHasShadow(){calls.push('shadow')} setVisibleOnAllWorkspaces(){calls.push('spaces')}
  setMaximumSize(){calls.push('max')} setMinimumSize(){calls.push('min')} setSize(){calls.push('size')}
  setPosition(){calls.push('position')} getBounds(){return{x:0,y:0,width:760,height:720}}
 }
 const area={x:0,y:0,width:1440,height:900};
 const electron={app:{setName(){},getPath(){return'/tmp'},setPath(){},whenReady(){return{then:fn=>fn()}},dock:{hide(){}},quit(){}},BrowserWindow:Window,ipcMain:{handle(){}},screen:{getCursorScreenPoint:()=>({x:0,y:0}),getDisplayNearestPoint:()=>({workArea:area})},session:{defaultSession:{setPermissionRequestHandler(){},setPermissionCheckHandler(){}}}};
 vm.runInNewContext('(function(){'+fs.readFileSync(require.resolve('../electron/onboarding.cjs'),'utf8')+'})()',{require:name=>name==='electron'?electron:require('node:module').createRequire(require.resolve('../electron/onboarding.cjs'))(name),__dirname:path.resolve(__dirname,'../electron'),module:{exports:{}},process:{versions:{electron:'fixture'},env:{},argv:[],stdin,stdout},console});
 const send=status=>stdin.emit('data',JSON.stringify({type:'state',step:'permissions',rows:[{permission:'microphone',status,required:true}],ready:false})+'\n');
 send('not-determined');const initial=calls.slice();send('granted');
 assert.equal(calls.filter(x=>x==='state').length,2);
 assert.equal(calls.filter(x=>x==='show').length,1,'permission refresh must not reactivate setup over OS prompts');
 assert.equal(calls.filter(x=>x==='size').length,1,'same mode must not resize its native window again');
 assert.equal(calls.filter(x=>x==='position').length,1,'same mode must preserve a user-moved window');
 stdin.emit('data','{"type":"show"}\n');
 assert.equal(calls.filter(x=>x==='show').length,2,'explicit parent show may reactivate setup');
});

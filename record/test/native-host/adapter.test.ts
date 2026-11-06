import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { NativeHostPortInput } from '../../src/backend/host/native/index.ts';
import { createRecordNativePort } from '../../src/backend/host/native/index.ts';
import { createRecordCapabilityProvider } from '../../src/backend/capabilities/provider.ts';
import type { RecordHostRuntime } from '../../src/backend/host/runtime.ts';

const snap = (revision = 3, epoch = 'desktop-a'): Awaited<ReturnType<NativeHostPortInput['readSnapshot']>> => ({ api: 1 as const, processEpoch: epoch, revision,
  scope: 'running-desktop-process' as const, freshness: 'cached-gui-observation' as const,
  permissions: [{ id: 'microphone', status: 'granted' as const, usableNow: false, relaunchNeeded: true, actions: ['open-system-settings' as const] }],
  shortcuts: { record: { configured: { keyCode: 83, modifiers: 2560, label: '⌥⇧S' }, effective: null,
    registration: 'unknown' as const, canValidate: true, canSave: false }, paste: { configured: null, effective: null, registration: 'unavailable' as const, canValidate: false, canSave: false } } });
function runtime(): RecordHostRuntime {
  const state = { phase: 'idle' as const, readiness: { ready: false, reason: 'fixture' } };
  return { state, startup: { state: 'ready', knownRequestIds: [] }, subscribe: () => () => undefined, dispatch: async () => state,
    reconcileStartup: async () => ({ state: 'ready', knownRequestIds: [] }), unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }),
    dispose: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }) } as unknown as RecordHostRuntime;
}
function command(client: ReturnType<typeof createRecordCapabilityProvider>, type: string, fields: Record<string, unknown> = {}) {
  const state = client.current(); return client.execute({ type, ...fields, requestId: `r-${Math.random()}`, expected: { providerEpoch: state.providerEpoch, revision: state.revision } } as never);
}
const until = async (predicate: () => boolean) => { for (let i = 0; i < 30 && !predicate(); i++) await new Promise(resolve => setTimeout(resolve, 0)); assert.ok(predicate()); };

test('maps cached permission and configured/effective registration facts without conflation', async () => {
  const port = { readSnapshot: async () => snap() } as NativeHostPortInput;
  const client = createRecordCapabilityProvider({ runtime: runtime(), native: createRecordNativePort(port, { requiredPermissionIds: ['microphone', 'camera'] }), providerEpoch: 'record' });
  await until(() => client.current().readiness.requirements.some(x => x.id === 'microphone'));
  const current = client.current();
  const mic = current.readiness.requirements.find(x => x.id === 'microphone')!;
  assert.equal(mic.status, 'granted'); assert.equal(mic.usableNow, false); assert.equal(mic.relaunchNeeded, true);
  assert.equal(client.current().readiness.requirements.find(x => x.id === 'camera')?.status, 'unavailable');
  assert.equal(current.shortcut.configured, '⌥⇧S'); assert.equal(current.shortcut.effective, null); assert.equal(current.shortcut.registration, 'unknown');
  client.detach();
});

test('edits validate returned candidate, cancel does not save, and stale host revision invalidates edit', async () => {
  let current = snap(); const calls: string[] = [];
  const port: NativeHostPortInput = { readSnapshot: async () => current,
    requestPermission: async () => ({ status: 'accepted' }), openSystemSettings: async () => ({ status: 'accepted' }),
    validateRecordShortcut: async (key, modifiers) => { calls.push(`validate:${key}:${modifiers}`); return { status: 'accepted', shortcut: { keyCode: 82, modifiers, label: '⌥⇧R' } }; },
    saveRecordShortcut: async shortcut => { calls.push(`save:${shortcut.keyCode}:${shortcut.label}`); return { status: 'accepted' }; } };
  const native = createRecordNativePort(port, { requiredPermissionIds: [] });
  const client = createRecordCapabilityProvider({ runtime: runtime(), native, providerEpoch: 'record' });
  await until(() => client.current().shortcut.configured === '⌥⇧S');
  const resBegin = await command(client, 'shortcut-begin-edit'); assert.equal(resBegin.status, 'accepted'); await until(() => !!client.current().shortcut.draft);
  const session = client.current().shortcut.draft!.sessionId;
  for (const value of ['Ctrl++R','constructor+R','__proto__+R','toString+R']) {
    const invalid = await command(client, 'shortcut-submit', { sessionId: session, value }); assert.equal(invalid.status, 'refused'); assert.deepEqual(calls, []);
  }
  await command(client, 'shortcut-submit', { sessionId: session, value: 'Alt+Shift+R' });
  await until(() => calls.length === 1);
  // Validation result keycode and label are used verbatim; cancel never writes.
  await command(client, 'shortcut-cancel', { sessionId: session }); assert.equal(calls.length, 1);
  await command(client, 'shortcut-begin-edit'); await until(() => !!client.current().shortcut.draft);
  const next = client.current().shortcut.draft!.sessionId;
  await command(client, 'shortcut-submit', { sessionId: next, value: 'Alt+Shift+R' }); await until(() => calls.length === 2);
  current = snap(4);
  // This is an external producer revision. A read invalidates the old candidate before save.
  await command(client, 'refresh'); await until(() => !client.current().shortcut.draft);
  const result = await command(client, 'shortcut-save', { sessionId: next }); assert.equal(result.status, 'refused'); assert.equal(calls.length, 2);
  client.detach();
});

test('passes the accepted epoch/revision exactly and never retries unknown mutation', async () => {
  let count = 0; const seen: unknown[] = [];
  const observed = snap(); observed.permissions[0].actions = ['request-permission'];
  const port = { readSnapshot: async () => observed, requestPermission: async (_id: string, state: unknown) => { seen.push(state); count++; return { status: 'outcome-unknown' as const }; },
    openSystemSettings: async () => ({ status: 'accepted' as const }), validateRecordShortcut: async () => ({ status: 'outcome-unknown' as const }), saveRecordShortcut: async () => ({ status: 'accepted' as const }) } satisfies NativeHostPortInput;
  const client = createRecordCapabilityProvider({ runtime: runtime(), native: createRecordNativePort(port, { requiredPermissionIds: ['microphone'] }), providerEpoch: 'record' });
  await until(() => client.current().readiness.requirements.some(x => x.id === 'microphone'));
  const result = await command(client, 'request-permission', { requirementId: 'microphone' });
  assert.equal(result.status, 'outcome-unknown'); assert.equal(count, 1); assert.deepEqual(seen, [{ processEpoch: 'desktop-a', revision: 3 }]);
  client.detach();
});

test('saves only the native validation result and leaves readback to a later snapshot', async () => {
  const initial = snap(); initial.shortcuts.record.canSave = true; initial.shortcuts.record.registration = 'registered';
  const saved: unknown[] = [];
  const port: NativeHostPortInput = { readSnapshot: async () => initial,
    requestPermission: async () => ({ status: 'accepted' }), openSystemSettings: async () => ({ status: 'accepted' }),
    validateRecordShortcut: async (_key, modifiers) => ({ status: 'accepted', shortcut: { keyCode: 82, modifiers, label: '⌥⇧R' } }),
    saveRecordShortcut: async shortcut => { saved.push(shortcut); return { status: 'refused', reason: 'registration-refused' }; } };
  const client = createRecordCapabilityProvider({ runtime: runtime(), native: createRecordNativePort(port, { requiredPermissionIds: [] }), providerEpoch: 'record-save' });
  await until(() => client.current().shortcut.configured === '⌥⇧S');
  await command(client, 'shortcut-begin-edit'); await until(() => !!client.current().shortcut.draft);
  const sessionId = client.current().shortcut.draft!.sessionId;
  await command(client, 'shortcut-submit', { sessionId, value: 'Alt+Shift+R' });
  await until(() => !!client.current().shortcut.draft && client.current().shortcut.draft!.value === 'Alt+Shift+R');
  const result = await command(client, 'shortcut-save', { sessionId });
  assert.equal(result.status, 'refused'); assert.deepEqual(saved, [{ keyCode: 82, modifiers: 2560, label: '⌥⇧R' }]);
  assert.equal(client.current().shortcut.configured, '⌥⇧S'); assert.equal(client.current().shortcut.effective, null);
  client.detach();
});


function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
function supportedPort(readSnapshot: NativeHostPortInput['readSnapshot']): NativeHostPortInput {
  return { readSnapshot, requestPermission: async () => ({status: 'accepted'}), openSystemSettings: async () => ({status: 'accepted'}),
    validateRecordShortcut: async () => ({status:'refused',reason:'unavailable'}), saveRecordShortcut: async () => ({status:'refused',reason:'unavailable'}) };
}

test('accepted save of the already effective shortcut closes only after authoritative readback', async () => {
  const current = snap(); current.shortcuts.record.canSave = true; current.shortcuts.record.registration = 'registered';
  current.shortcuts.record.effective = structuredClone(current.shortcuts.record.configured);
  const port = supportedPort(async () => current); let saves = 0;
  port.validateRecordShortcut = async () => ({status:'accepted',shortcut:structuredClone(current.shortcuts.record.configured!)});
  port.saveRecordShortcut = async () => { saves++; return {status:'accepted'}; };
  const client = createRecordCapabilityProvider({runtime:runtime(),native:createRecordNativePort(port,{requiredPermissionIds:[]}),providerEpoch:'same-value'});
  await until(() => client.current().shortcut.effective === '⌥⇧S');
  await command(client,'shortcut-begin-edit'); const sessionId = client.current().shortcut.draft!.sessionId;
  await command(client,'shortcut-submit',{sessionId,value:'Alt+Shift+S'});
  assert.equal((await command(client,'shortcut-save',{sessionId})).status,'accepted');
  assert.equal(saves,1); assert.equal(client.current().shortcut.draft,null);
  assert.equal(client.current().shortcut.effective,'⌥⇧S'); client.detach();
});

test('producer mutation cannot rewrite the validated shortcut candidate', async () => {
  const current = snap(); current.shortcuts.record.canSave = true; current.shortcuts.record.registration = 'registered';
  const candidate = {keyCode:82,modifiers:2560,label:'⌥⇧R'};
  const port = supportedPort(async () => current); const saved: unknown[] = [];
  port.validateRecordShortcut = async () => ({status:'accepted',shortcut:candidate});
  port.saveRecordShortcut = async value => { saved.push(structuredClone(value)); return {status:'accepted'}; };
  const client = createRecordCapabilityProvider({runtime:runtime(),native:createRecordNativePort(port,{requiredPermissionIds:[]}),providerEpoch:'candidate'});
  await until(() => client.current().shortcut.configured === '⌥⇧S');
  await command(client,'shortcut-begin-edit'); const sessionId = client.current().shortcut.draft!.sessionId;
  await command(client,'shortcut-submit',{sessionId,value:'Alt+Shift+R'});
  candidate.keyCode=99; candidate.label='changed after validation';
  await command(client,'shortcut-save',{sessionId});
  assert.deepEqual(saved,[{keyCode:82,modifiers:2560,label:'⌥⇧R'}]);
  assert.equal(client.current().shortcut.effective,null); assert.ok(client.current().shortcut.draft,'accepted dispatch does not invent changed readback');
  client.detach();
});

test('deferred provider refresh cannot rebase a permission effect onto an unobserved native revision', async () => {
  let current = snap(); current.permissions[0]!.actions=['request-permission'];
  let notify!:()=>void, reads=0, actions=0;
  const blocked = deferred<{version:1;providerEpoch:string;revision:number;captures:[];nextShortcutAdmission:{state:'unavailable'}}>();
  const output = {version:1 as const,providerEpoch:'output',revision:0,captures:[] as [],nextShortcutAdmission:{state:'unavailable' as const}};
  const port=supportedPort(async()=>current); port.requestPermission=async()=>{actions++;return{status:'accepted'};};
  const client=createRecordCapabilityProvider({runtime:runtime(),native:createRecordNativePort(port,{requiredPermissionIds:[]}),providerEpoch:'refresh-race',
    captureFacts:{readFacts:async()=>++reads===2?blocked.promise:output,observe:listener=>{notify=listener;return()=>{};}}});
  await until(()=>client.current().readiness.requirements.some(x=>x.id==='microphone'));
  current=snap(4); current.permissions[0]!.actions=['request-permission']; notify();
  await until(()=>reads===2);
  const result=await command(client,'request-permission',{requirementId:'microphone'});
  assert.equal(result.status,'refused'); assert.equal(actions,0);
  blocked.resolve(output); client.detach();
});

test('retired GUI epochs cannot return as new authority, and deferred commands are snapshotted', async () => {
  let current=snap(); current.permissions[0]!.actions=['request-permission'];
  const held=deferred<{status:'accepted'}>(); const entered=deferred<void>(); const ids:string[]=[];
  const port=supportedPort(async()=>current);
  port.requestPermission=async(id)=>{ids.push(id); if(ids.length===1){entered.resolve();return held.promise;}return{status:'accepted'};};
  const native=createRecordNativePort(port,{requiredPermissionIds:[]}); const facts=await native.readFacts();
  const first=native.dispatch!({type:'request-permission',requirementId:'microphone',requestId:'first',expected:{providerEpoch:'record',revision:0}},facts);
  await entered.promise;
  const action={type:'request-permission' as const,requirementId:'microphone',requestId:'second',expected:{providerEpoch:'record',revision:0}};
  const second=native.dispatch!(action,facts); action.requirementId='camera'; held.resolve({status:'accepted'});
  assert.equal(await first,'accepted'); assert.equal(await second,'accepted'); assert.deepEqual(ids,['microphone','microphone']);
  current=snap(0,'desktop-b'); await native.readFacts(); current=snap(9,'desktop-a');
  await assert.rejects(native.readFacts(),/retired/); native.dispose!();
});

test('native cleanup failure cannot prevent runtime detachment', async () => {
  const host=runtime(); let detached=false;
  host.dispose=()=>{detached=true;return{requestIds:[],storageUnavailable:false,reconciliation:'complete'};};
  const native=createRecordNativePort(supportedPort(async()=>snap()),{requiredPermissionIds:[]});
  native.dispose=()=>{throw new Error('injected cleanup failure');};
  const client=createRecordCapabilityProvider({runtime:host,native,providerEpoch:'cleanup'});
  assert.throws(()=>client.detach(),/cleanup failure/); assert.equal(detached,true);
});


test('native refusal remains a complete public RecordClient result', async () => {
  const current=snap(); current.shortcuts.record.canValidate=false;
  const native=createRecordNativePort(supportedPort(async()=>current),{requiredPermissionIds:[]});
  const client=createRecordCapabilityProvider({runtime:runtime(),native,providerEpoch:'refusal-contract'});
  await until(()=>client.current().shortcut.configured==='⌥⇧S');
  const result=await command(client,'shortcut-begin-edit');
  assert.equal(result.status,'refused');
  if(result.status==='refused') assert.equal(typeof result.reason,'string');
  assert.equal(client.current().shortcut.draft,null); client.detach();
});


test('changed facts at the same native token cannot become hidden effect authority', async () => {
  let current=snap(); let effects=0;
  const port=supportedPort(async()=>current);
  port.requestPermission=async()=>{effects++;return{status:'accepted'};};
  const native=createRecordNativePort(port,{requiredPermissionIds:[]});
  const original=await native.readFacts();
  current=structuredClone(current); current.permissions[0]!.actions=['request-permission'];
  await assert.rejects(native.readFacts(),/without a new observation/);
  const action={type:'request-permission' as const,requirementId:'microphone',requestId:'hidden-authority',expected:{providerEpoch:'record',revision:0}};
  assert.equal(await native.dispatch!(action,original),'refused'); assert.equal(effects,0);
  current=snap(); // Returning the old bytes cannot unpoison the reused token.
  await assert.rejects(native.readFacts(),/inconsistent/);
  current=snap(4); current.permissions[0]!.actions=['request-permission'];
  const fresh=await native.readFacts();
  assert.equal(await native.dispatch!(action,fresh),'accepted'); assert.equal(effects,1);
  native.dispose!();
});


test('malformed runtime snapshots disable direct adapter effects until a valid observation', async () => {
  let supplied: unknown=snap(); let effects=0;
  const port=supportedPort(async()=>supplied as Awaited<ReturnType<NativeHostPortInput['readSnapshot']>>);
  port.openSystemSettings=async()=>{effects++;return{status:'accepted'};};
  const native=createRecordNativePort(port,{requiredPermissionIds:[]}); const original=await native.readFacts();
  const malformed=[
    {...snap(4),permissions:[{...snap(4).permissions[0],actions:'open-system-settings'}]},
    {...snap(4),permissions:[{...snap(4).permissions[0],status:'not-a-status'}]},
    {...snap(4),shortcuts:{...snap(4).shortcuts,record:{...snap(4).shortcuts.record,canValidate:'yes'}}},
    {...snap(4),permissions:[...snap(4).permissions,...snap(4).permissions]},
    {...snap(4),permissions:[{...snap(4).permissions[0],status:Object('granted')}]},
    {...snap(4),shortcuts:{...snap(4).shortcuts,record:{...snap(4).shortcuts.record,registration:Object('externally-managed')}}},
    {...snap(4),permissions:Array(1)},
  ];
  const action={type:'open-system-settings' as const,requirementId:'microphone',requestId:'malformed',expected:{providerEpoch:'record',revision:0}};
  for(const value of malformed){
    supplied=value; await assert.rejects(native.readFacts(),/unavailable/);
    assert.equal(await native.dispatch!(action,original),'refused'); assert.equal(effects,0);
  }
  supplied=snap(); const valid=await native.readFacts();
  assert.equal(await native.dispatch!(action,valid),'accepted'); assert.equal(effects,1); native.dispose!();
});


test('failed replacement validation cannot retain the previous save candidate', async () => {
  const current=snap(); current.shortcuts.record.registration='registered'; current.shortcuts.record.canSave=true;
  const port=supportedPort(async()=>current); let validates=0,saves=0;
  port.validateRecordShortcut=async()=>{if(++validates===2)throw new Error('lost validation reply');return{status:'accepted',shortcut:{keyCode:82,modifiers:2560,label:'⌥⇧R'}};};
  port.saveRecordShortcut=async()=>{saves++;return{status:'accepted'};};
  const client=createRecordCapabilityProvider({runtime:runtime(),native:createRecordNativePort(port,{requiredPermissionIds:[]}),providerEpoch:'replacement-validation'});
  await until(()=>client.current().shortcut.configured==='⌥⇧S');
  await command(client,'shortcut-begin-edit');const sessionId=client.current().shortcut.draft!.sessionId;
  await command(client,'shortcut-submit',{sessionId,value:'Alt+Shift+R'});
  assert.equal((await command(client,'shortcut-submit',{sessionId,value:'Alt+Shift+T'})).status,'outcome-unknown');
  assert.equal(client.current().shortcut.draft?.value,'Alt+Shift+T');
  assert.equal((await command(client,'shortcut-save',{sessionId})).status,'refused');assert.equal(saves,0);
  assert.equal(validates,2);client.detach();
});

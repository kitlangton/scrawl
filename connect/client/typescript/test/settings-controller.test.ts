import assert from 'node:assert/strict';
import { test } from 'node:test';
import { AccountController } from '../dist/settings-controller.cjs';
import type { AccountBackend, Approval, Health, LocalState } from '../src/settings-controller.cts';
import type { SafeError } from '../src/settings-presentation.ts';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
class Fake implements AccountBackend {
  local: LocalState = { snapshot: {server:null,connected:false,uploadsEnabled:false,
    renewalRequired:false,reauthorizationRequired:false,accountInitial:null}, error:null,sharedLinksUrl:null };
  loginCalls = 0; activations = 0; disables: boolean[] = [];
  loginResult = deferred<SafeError|null>();
  disableResult: ReturnType<typeof deferred<SafeError|null>> | null = null;
  approval?: (value: Approval) => void;
  signal?: AbortSignal;
  async read() { return structuredClone(this.local); }
  async login(_server:string, signal:AbortSignal, approval:(value:Approval)=>void) {
    this.loginCalls += 1; this.signal=signal; this.approval=approval;
    return this.loginResult.promise;
  }
  async probe():Promise<Health> { return 'online'; }
  async disable(logout:boolean) {
    this.disables.push(logout);
    if (this.disableResult) return this.disableResult.promise;
    if (this.local.snapshot) this.local.snapshot.uploadsEnabled=false;
    return null;
  }
  async activate() { this.activations += 1; return true; }
  approved() {
    this.local.snapshot = {...this.local.snapshot!,server:'https://hosted.tryapparent.com',connected:true,uploadsEnabled:true};
    this.loginResult.resolve(null);
  }
}
async function start() {
  const backend=new Fake(), controller=new AccountController(backend);
  await controller.request(); await controller.request('connect'); await controller.request('allow');
  return {backend,controller};
}

test('only explicit consent starts login; successful receipt needs an admitted read before activation', async () => {
  const backend=new Fake(), controller=new AccountController(backend);
  await controller.request('allow'); assert.equal(backend.loginCalls,0);
  await controller.request('connect');
  assert.equal(controller.view().presentationState.consent,true);
  await controller.request('allow'); await controller.request('allow');
  assert.equal(backend.loginCalls,1);
  backend.approval!({verificationUri:'https://connect.test/device',userCode:'TEST-CODE'});
  await controller.request(); assert.equal(controller.view().userCode,'TEST-CODE');
  backend.approved(); await Promise.resolve(); await Promise.resolve();
  assert.equal(backend.activations,0);
  await controller.request(); assert.equal(backend.activations,1);
  await controller.request(); assert.equal(backend.activations,1);
});

test('cancel wins over a queued successful login; stale completion never activates', async () => {
  const {backend,controller}=await start();
  backend.approved(); await Promise.resolve(); await Promise.resolve();
  await controller.request('cancel');
  assert.equal(backend.signal!.aborted,true);
  assert.deepEqual(backend.disables,[false]);
  assert.equal(backend.activations,0);
  assert.equal(controller.view().presentationState.snapshot!.uploadsEnabled,false);
});

test('close before terminal receipt consumption revokes and retains durable disable ownership', async () => {
  const {backend,controller}=await start();
  backend.disableResult=deferred();
  backend.approved(); await Promise.resolve(); await Promise.resolve();
  controller.close(); controller.close();
  assert.deepEqual(backend.disables,[false]);
  assert.equal(backend.activations,0);
  assert.equal(controller.view().presentationState.closed,true);
  assert.ok(controller.view().buttons.every(b=>!b.enabled));
  backend.disableResult.resolve(null);
});

for (const error of ['rejected','expired','offline'] as const) {
  test(`login ${error} remains a closed safe error and can be retried`, async () => {
    const {backend,controller}=await start(); backend.loginResult.resolve(error);
    await Promise.resolve(); await Promise.resolve();
    await controller.request();
    assert.equal(controller.view().presentationState.error,error);
    assert.equal(backend.activations,0);
    await controller.request('connect');
    assert.equal(controller.view().presentationState.consent,true);
  });
}

test('unverified login readback triggers durable cleanup rather than activation', async () => {
  const {backend,controller}=await start();
  backend.approved(); backend.local.snapshot!.server='https://other.test';
  await Promise.resolve(); await Promise.resolve();
  await controller.request(); await controller.request();
  assert.equal(backend.activations,0);
  assert.deepEqual(backend.disables,[false]);
  assert.equal(controller.view().presentationState.error,'unverified-commit');
});

test('failed disable preserves direct retry controls even when local readback fails', async () => {
  const backend=new Fake(), controller=new AccountController(backend);
  backend.disableResult=deferred();
  await controller.request(); await controller.request('disconnect');
  backend.local={snapshot:null,error:'local-state',sharedLinksUrl:null};
  backend.disableResult.resolve('local-state');
  await Promise.resolve(); await Promise.resolve(); await controller.request();
  assert.equal(controller.view().presentationState.retryDisable,true);
  assert.ok(controller.view().buttons.some(b=>b.command==='disconnect' && b.enabled));
  await controller.request('connect');
  assert.equal(controller.view().presentationState.consent,false);
});

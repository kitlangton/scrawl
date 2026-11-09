import { test } from 'node:test';
import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { shared } from './testing/sdk.ts';
import { sign } from './controller.ts';
const hooks = registerHooks({ resolve(specifier, context, next) {
  if (['@hermes/plugin-sdk', 'react/jsx-runtime'].includes(specifier)) return { url: new URL('./testing/sdk.ts', import.meta.url).href, shortCircuit: true };
  return next(specifier, context);
} });
const plugin = (await import('./plugin.ts')).default;
hooks.deregister();
async function until(predicate: () => unknown) {
  const deadline = Date.now() + 1500;
  while (!predicate()) { if (Date.now() > deadline) throw new Error('Async test condition did not settle'); await new Promise(resolve => setTimeout(resolve, 2)); }
}
function fixture(external = true, stored: any = null) {
  let contribution: any, discard: any, interval: (() => void) | undefined, saved: any, launch: string | undefined;
  let dispose = () => {};
  const ctx = { onDispose: (fn: () => void) => { dispose = fn; }, setInterval: (fn: () => void) => { interval = fn; return () => { interval = undefined; }; },
    storage: { get: () => stored, set: (_key: string, value: unknown) => { saved = structuredClone(value); } },
    os: { openExternal: async (url: string) => { launch = url; return external; } }, register: (c: any) => { if (c.id === 'record') contribution = c; else discard = c; } };
  plugin.register(ctx);
  return { click: () => contribution.render().props.onClick(), discard: () => discard.data.run(), tick: () => interval?.(), saved: () => saved, launch: () => launch, watching: () => Boolean(interval), dispose: () => dispose() };
}
async function packet(secret: string, phase: string) {
  const payload = JSON.stringify({ phase, ...(phase === 'saved' ? { source: '/private/owned.apparent' } : {}) });
  return Response.json({ payload, mac: await sign(secret, payload) });
}
function reset() {
  shared.submissions = []; shared.failSubmit = false; shared.session = 'runtime-A'; shared.owner = { connectionId: 'local-A', profile: 'work' };
  shared.routes = [{ connectionId: 'local-A', profile: 'work', targetProfile: 'work', mode: 'local' }];
  shared.runtime.instance_id = 'runtime-instance-A';
  shared.launchWait = null; shared.keyReads = []; shared.failLaunch = false;
}
test('completion submits once to captured gateway/session after foreground switches; next click records again', async () => {
  reset(); const originalFetch = globalThis.fetch;
  let phase = 'recording'; let acknowledgements = 0; const f = fixture();
  globalThis.fetch = async url => {
    if (String(url).includes('/ack/')) { acknowledgements++; return Response.json({ ok: true }); }
    return packet(shared.key, phase);
  };
  try {
    f.click(); await until(() => f.launch());
    assert.match(f.launch()!, /^apparent:\/\/hermes\/[a-f0-9]{32}\/\d+$/);
    const first = f.saved();
    shared.session = 'runtime-B'; shared.owner = { connectionId: 'other-gateway', profile: 'work' };
    phase = 'saved'; f.tick(); await until(() => acknowledgements === 1);
    f.tick(); await new Promise(resolve => setTimeout(resolve, 5));
    assert.equal(shared.submissions.length, 1);
    assert.equal((shared.submissions[0] as any).params.session_id, 'runtime-A');
    assert.equal((shared.submissions[0] as any).route.connectionId, 'local-A');
    assert.equal((shared.submissions[0] as any).params.input_id, first.token);
    assert.equal(shared.keyReads.length, 1);
    assert.equal(shared.keyReads[0].route.connectionId, 'local-A');
    assert.equal(shared.keyReads[0].params.session_id, 'runtime-A');
    assert.ok(!JSON.stringify(f.saved()).includes(shared.key));
    shared.owner = { connectionId: 'local-A', profile: 'work' }; phase = 'recording';
    f.click(); await until(() => f.saved()?.token && f.saved().token !== first.token);
  } finally { globalThis.fetch = originalFetch; }
});
test('lost submit replies retry only original input_id and stop; no title lookup', async () => {
  reset(); shared.failSubmit = true; const originalFetch = globalThis.fetch; const f = fixture();
  globalThis.fetch = async () => packet(shared.key, 'saved');
  try {
    f.click(); await until(() => f.launch()); f.tick(); await until(() => !f.watching());
    assert.equal(shared.submissions.length, 2);
    assert.deepEqual(shared.submissions[0], shared.submissions[1]);
    assert.equal((shared.submissions[0] as any).method, 'prompt.submit');
    f.tick(); assert.equal(shared.submissions.length, 2);
  } finally { shared.failSubmit = false; globalThis.fetch = originalFetch; }
});
test('a listener knowing the public launch ID cannot forge completion', async () => {
  reset(); const originalFetch = globalThis.fetch; const f = fixture(); let requested: string | undefined;
  globalThis.fetch = async url => { requested = String(url); return packet(f.launch()!.split('/')[3], 'saved'); };
  try {
    f.click(); await until(() => f.launch()); f.tick(); await until(() => requested);
    await new Promise(resolve => setTimeout(resolve, 10));
    assert.ok(!requested!.includes(shared.key));
    assert.equal(shared.submissions.length, 0);
    assert.equal(f.saved().source, undefined);
  } finally { globalThis.fetch = originalFetch; }
});
test('changed route mapping or replaced runtime cannot receive the saved recording', async () => {
  const originalFetch = globalThis.fetch;
  try {
    for (const change of [() => { shared.routes[0].targetProfile = 'other'; }, () => { shared.runtime.instance_id = 'replacement'; }]) {
      reset(); const f = fixture();
      let phase = 'recording';
      globalThis.fetch = async () => packet(shared.key, phase);
      f.click(); await until(() => f.launch()); f.tick(); await until(() => shared.keyReads.length === 1);
      await new Promise(resolve => setTimeout(resolve, 10));
      change(); phase = 'saved'; f.tick(); await until(() => !f.watching());
      assert.equal(shared.submissions.length, 0);
      assert.equal(f.saved().source, '/private/owned.apparent');
      await f.discard(); assert.equal(f.saved(), null);
    }
  } finally { reset(); globalThis.fetch = originalFetch; }
});
test('stock app-link refusal uses only the original local session safe shell; remote routes never execute', async () => {
  reset();
  const f = fixture(false);
  f.click(); await until(() => f.watching());
  assert.equal(shared.submissions.length, 1);
  const call = shared.submissions[0] as any;
  assert.equal(call.method, 'shell.exec');
  assert.equal(call.route.connectionId, 'local-A');
  assert.equal(call.params.session_id, 'runtime-A');
  assert.ok(call.params.command.includes(f.launch()));
  reset(); shared.routes[0].mode = 'ssh';
  const remote = fixture(false); remote.click();
  await until(() => remote.saved() === null);
  assert.equal(remote.launch(), undefined);
  assert.equal(shared.submissions.length, 0);
  assert.equal(remote.watching(), false);
  reset();
});
test('a lost local launch reply retains the original pending handoff without launching again', async () => {
  reset(); shared.failLaunch = true;
  const f = fixture(false); f.click();
  await until(() => f.watching());
  assert.ok(f.saved().token);
  assert.equal(shared.submissions.length, 1);
  f.click(); await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal(shared.submissions.length, 1);
  f.dispose(); reset();
});
test('unloading during local launch leaves recovery persisted and never creates a timer afterward', async () => {
  reset(); let finish!: () => void;
  shared.launchWait = new Promise<void>(resolve => { finish = resolve; });
  const f = fixture(false); f.click();
  await until(() => shared.submissions.length === 1);
  f.dispose(); finish(); await new Promise(resolve => setTimeout(resolve, 5));
  assert.ok(f.saved().token);
  assert.equal(f.watching(), false);
  reset();
});
test('old pending sources cannot bypass the private-key protocol after reload', async () => {
  reset();
  const old = { token: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', port: 45000, created: Date.now(), source: '/private/old.apparent',
    destination: { session: 'runtime-A', connectionId: 'local-A', profile: 'work', targetProfile: 'work', mode: 'local', local: true }, runtime: shared.runtime };
  const f = fixture(true, old);
  await until(() => f.saved() === null);
  assert.equal(f.watching(), false);
  assert.equal(shared.submissions.length, 0);
  assert.equal(shared.keyReads.length, 0);
});
test('reload obtains the private key again and retries the same saved handoff without launching', async () => {
  reset(); shared.failSubmit = true;
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async url => String(url).includes('/ack/') ? Response.json({ ok: true }) : packet(shared.key, 'saved');
  try {
    const first = fixture(); first.click(); await until(() => first.launch()); first.tick();
    await until(() => !first.watching());
    const stored = first.saved(); first.dispose();
    assert.ok(!JSON.stringify(stored).includes(shared.key));
    reset();
    const resumed = fixture(true, stored);
    await until(() => resumed.saved() === null);
    assert.equal(resumed.launch(), undefined);
    assert.equal(shared.keyReads.length, 1);
    assert.equal(shared.submissions.length, 1);
    assert.equal((shared.submissions[0] as any).params.input_id, stored.token);
    assert.equal((shared.submissions[0] as any).params.session_id, stored.destination.session);
    resumed.dispose();
  } finally { reset(); globalThis.fetch = originalFetch; }
});

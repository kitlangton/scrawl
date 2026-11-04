import assert from 'node:assert/strict';
import test from 'node:test';
import { authorizeDevice, DeviceFlowError } from '../dist/device-flow.cjs';
import type { DeviceTransport } from '../dist/device-flow.cjs';

const start = (patch: Record<string, unknown> = {}) => ({
  deviceCode: 'private-device-secret', userCode: 'ABCD-1234',
  verificationUri: 'https://accounts.example/connect/device?code=ABCD-1234',
  expiresIn: 60, interval: 1, ...patch,
});
const approved = (patch: Record<string, unknown> = {}) => ({ status: 'approved', scope: 'upload',
  accessToken: 'access-secret', refreshToken: 'refresh-secret', grantId: 'grant-id',
  expiresIn: 300, refreshExpiresIn: 3600, ...patch });
function fixture(reply: unknown = approved()) {
  let polls = 0, commits: string[] = [];
  const transport: DeviceTransport = {
    start: async () => start(), poll: async () => { polls++; return reply; },
    commit: async origin => { commits.push(origin); return null; },
  };
  return { transport, polls: () => polls, commits };
}

test('validates approval URI before exposing only the public code and URI', async () => {
  const f = fixture(); let approval: unknown;
  const result = await authorizeDevice(f.transport, new AbortController().signal, value => { approval = value; },
    { now: () => 0, sleep: async () => {} });
  assert.equal(result, null);
  assert.deepEqual(approval, { verificationUri: start().verificationUri, userCode: 'ABCD-1234' });
  assert.deepEqual(f.commits, ['https://accounts.example']);
  assert.equal(JSON.stringify(approval).includes('private-device-secret'), false);
});

for (const uri of [
  'file:///tmp/private-device-secret', 'https://user:pass@accounts.example/device',
  'https://accounts.example/device?code=ABCD-1234&next=x', 'https://accounts.example/%2f?code=ABCD-1234',
  'https://accounts.example/device#frag', 'http://evil.example/device',
]) test(`rejects unsafe approval URI ${uri}`, async () => {
  const f = fixture(); f.transport.start = async () => start({ verificationUri: uri });
  let exposed = false;
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => { exposed = true; }), 'unavailable');
  assert.equal(exposed, false); assert.equal(f.polls(), 0); assert.deepEqual(f.commits, []);
});

test('expiry zero reports expired without polling or committing', async () => {
  const f = fixture(); f.transport.start = async () => start({ expiresIn: 0 });
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => {}, { now: () => 10 }), 'expired');
  assert.equal(f.polls(), 0); assert.deepEqual(f.commits, []);
});

test('pending response then approved response commits once', async () => {
  const f = fixture(); let polls = 0;
  f.transport.poll = async (_code, timeout) => {
    assert.ok(timeout <= 30_000 && timeout > 0);
    return ++polls === 1 ? { status: 'pending' } : approved();
  };
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => {}, { now: () => 0, sleep: async () => {} }), null);
  assert.equal(polls, 2); assert.deepEqual(f.commits, ['https://accounts.example']);
});

test('abort while waiting stops before poll and commit', async () => {
  const f = fixture(); const controller = new AbortController();
  const waiting = authorizeDevice(f.transport, controller.signal, () => {}, {
    now: () => 0, sleep: async () => { controller.abort(); },
  });
  assert.equal(await waiting, 'cancelled'); assert.equal(f.polls(), 0); assert.deepEqual(f.commits, []);
});

test('invalid approved grant never commits and raw transport errors stay secret-free', async () => {
  const f = fixture(approved({ grantId: 'bad\nsecret' }));
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => {}, { now: () => 0, sleep: async () => {} }), 'unavailable');
  assert.deepEqual(f.commits, []);
  f.transport.start = async () => { throw new Error('private-device-secret'); };
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => {}), 'unavailable');
});

test('only explicit safe transport errors cross the boundary', async () => {
  const f = fixture(); f.transport.start = async () => { throw new DeviceFlowError('offline'); };
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => {}), 'offline');
  f.transport.start = async () => start();
  f.transport.commit = async () => { throw new Error('access-secret'); };
  assert.equal(await authorizeDevice(f.transport, new AbortController().signal, () => {}, { now: () => 0, sleep: async () => {} }), 'unavailable');
});

for (const userCode of [undefined, null, 1234, {}]) test('malformed public code cannot be coerced into an approval', async () => {
  const f=fixture();f.transport.start=async()=>start({userCode});let exposed=false;
  assert.equal(await authorizeDevice(f.transport,new AbortController().signal,()=>{exposed=true;}),'unavailable');
  assert.equal(exposed,false);assert.equal(f.polls(),0);
});

test('server interval zero still waits one second and a late approval cannot commit',async()=>{
  const f=fixture();f.transport.start=async()=>start({interval:0,expiresIn:2});let clock=0;
  f.transport.poll=async()=>{clock=2000;return approved();};
  assert.equal(await authorizeDevice(f.transport,new AbortController().signal,()=>{}, {
    now:()=>clock,sleep:async ms=>{assert.equal(ms,1000);clock+=ms;},
  }),'expired');
  assert.deepEqual(f.commits,[]);
});

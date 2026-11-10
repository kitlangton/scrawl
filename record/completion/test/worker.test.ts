import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import {once} from 'node:events';
import type {Decision} from '../src/policy.ts';

const context = {capture:{generation:'7',captureId:'a',requestId:'request-a'},
  platform:'linux',source:'screen',agentRequested:false,automaticTrialEnabled:false,lab:false,hyprland:true,wayland:true};
const input = {version:1,context,current:true,outcome:{kind:'saved',bundlePath:'/synthetic/a.apparent'}};
for(const result of ['copied','failed','revoked']) {
  test(`real headless worker uses ${result} capability receipt without Electron`,async t=>{
    const child=spawn(process.execPath,['--experimental-strip-types',new URL('../src/worker.ts',import.meta.url).pathname],{stdio:['pipe','pipe','pipe']});
    t.after(()=>child.kill());
    const exit=once(child,'exit');
    const lines=createInterface({input:child.stdout})[Symbol.asyncIterator]();
    child.stdin.write(JSON.stringify(input)+'\n');
    const first: Decision=JSON.parse((await lines.next()).value!);
    assert.equal(first.state.status,'Saved');
    assert.equal(first.effects[0]?.type,'copy');
    child.stdin.write(JSON.stringify({result})+'\n');
    const final: Decision=JSON.parse((await lines.next()).value!);
    assert.deepEqual(final.effects,[]);
    assert.equal(final.state.observation?.clipboard,result==='revoked'?'notRequested':result);
    assert.equal(final.state.current,result!=='revoked');
    assert.deepEqual(await exit,[0,null]);
  });
}
test('worker failure/EOF cannot return copied or discard the initial saved fact',async t=>{
  const child=spawn(process.execPath,['--experimental-strip-types',new URL('../src/worker.ts',import.meta.url).pathname]);
  t.after(()=>child.kill());
  const exit=once(child,'exit');
  const lines=createInterface({input:child.stdout})[Symbol.asyncIterator]();
  child.stdin.write(JSON.stringify(input)+'\n');
  const saved: Decision=JSON.parse((await lines.next()).value!);
  child.stdin.end();
  assert.deepEqual(await exit,[1,null]);
  assert.deepEqual(saved.state.observation?.outcome,input.outcome);
  assert.equal(saved.state.observation?.clipboard,'notRequested');
});
test('failed finalizer completes without requesting any capability',async t=>{
  const child=spawn(process.execPath,['--experimental-strip-types',new URL('../src/worker.ts',import.meta.url).pathname]);
  t.after(()=>child.kill());
  const exit=once(child,'exit');
  const lines=createInterface({input:child.stdout})[Symbol.asyncIterator]();
  child.stdin.write(JSON.stringify({...input,outcome:{kind:'failed',message:'publication failed'}})+'\n');
  const failed: Decision=JSON.parse((await lines.next()).value!);
  assert.equal(failed.state.status,'Capture failed');
  assert.deepEqual(failed.effects,[]);
  assert.deepEqual(await exit,[0,null]);
});

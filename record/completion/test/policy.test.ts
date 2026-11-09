import {test} from 'node:test';
import assert from 'node:assert/strict';
import {begin, reduce} from '../src/policy.ts';
import type {Context, Event} from '../src/policy.ts';
const capture = {generation: '9007199254740993', captureId: 'capture-a', requestId: 'request-a'};
const context: Context = {capture, platform:'linux', source:'screen', agentRequested:false,
  automaticTrialEnabled:false, lab:false, hyprland:true, wayland:true};
const saved: Event = {type:'finalized',capture,outcome:{kind:'saved',bundlePath:'/synthetic/a.apparent'}};
const copied: Event = {type:'clipboard',capture,result:'copied'};

test('only a successful publication requests copy; only the actual receipt says copied', () => {
  const decision = reduce(begin(context,true),saved);
  assert.deepEqual(decision.effects,[{type:'copy',capture,mode:'bundle'}]);
  assert.equal(decision.state.status,'Saved');
  assert.equal(decision.state.observation?.clipboard,'notRequested');
  const done = reduce(decision.state,copied);
  assert.equal(done.state.status,'Saved · copied');
  assert.equal(done.state.observation?.clipboard,'copied');
  assert.deepEqual(done.effects,[]);
  assert.deepEqual(reduce(done.state,saved),{state:done.state,effects:[]});
  assert.deepEqual(reduce(done.state,{...copied,result:'failed'}),{state:done.state,effects:[]});
});
test('clipboard failure retains the exact saved artifact and offers manual access', () => {
  const state = reduce(reduce(begin(context,true),saved).state,{type:'clipboard',capture,result:'failed'}).state;
  assert.deepEqual(state.observation?.outcome,saved.outcome);
  assert.equal(state.status,'Saved · open bundle (copy failed)');
});
test('finalization failures never copy or become saved', () => {
  for(const message of ['publication failed','No screen frames were captured (privacy gate)']) {
    const decision = reduce(begin(context,true),{type:'finalized',capture,outcome:{kind:'failed',message}});
    assert.deepEqual(decision.effects,[]);
    assert.equal(decision.state.observation?.outcome.kind,'failed');
    assert.equal(decision.state.observation?.clipboard,'notRequested');
    assert.match(decision.state.status!,/^Capture failed/);
    assert.deepEqual(reduce(decision.state,copied),{state:decision.state,effects:[]});
    assert.deepEqual(reduce(decision.state,saved),{state:decision.state,effects:[]});
  }
});
for(const type of ['dismiss','cancel','revoke'] as const) {
  test(`${type} retains terminal receipt facts but cannot copy or update a consumer`,()=>{
    const decision = reduce(reduce(begin(context,true),{type}).state,saved);
    assert.deepEqual(decision.effects,[]);
    assert.deepEqual(decision.state.observation,{capture,outcome:saved.outcome,clipboard:'notRequested',current:false});
    const inFlight = reduce(reduce(begin(context,true),saved).state,{type});
    assert.deepEqual(reduce(inFlight.state,copied),{state:inFlight.state,effects:[]});
  });
}
test('out-of-order results and new generations cannot overwrite the current capture',()=>{
  const newer = begin({...context,capture:{...capture,generation:'9007199254740994'}},true);
  assert.deepEqual(reduce(newer,saved),{state:newer,effects:[]});
  for(const wrong of [{...capture,captureId:'b'},{...capture,requestId:'b'}]) {
    assert.deepEqual(reduce(begin(context,true),{...saved,capture:wrong}).effects,[]);
  }
  assert.equal(reduce(begin(context,false),saved).state.observation?.current,false);
  assert.deepEqual(reduce(begin(context,true),copied).effects,[]);
  assert.equal(reduce(begin(context,true),copied).state.observation,null);
});
test('disconnect/reconnect replays facts without repeating a potentially completed copy',()=>{
  const inFlight = reduce(begin(context,true),saved).state;
  const lost = reduce(inFlight,{type:'disconnect'}).state;
  const restored = reduce(lost,{type:'reconnect'});
  assert.deepEqual(restored.effects,[]);
  assert.equal(restored.state.status,'Saved');
  assert.deepEqual(reduce(restored.state,saved).effects,[]);
  assert.deepEqual(reduce(restored.state,copied).effects,[]);
  const disconnected = reduce(begin(context,true),{type:'disconnect'}).state;
  assert.deepEqual(reduce(disconnected,saved).effects,[]);
});
test('blocked automatic trial is text-only and manual; other existing paths use bundle capability',()=>{
  const enabled = {...context,automaticTrialEnabled:true};
  const manual = reduce(begin(enabled,true),saved);
  assert.equal(manual.effects[0]?.mode,'text');
  assert.equal(reduce(manual.state,copied).state.status,'Saved · paste manually');
  for(const change of [{agentRequested:true},{lab:true},{hyprland:false},{wayland:false},{source:'phone' as const},{platform:'macos' as const}]) {
    assert.equal(reduce(begin({...enabled,...change},true),saved).effects[0]?.mode,'bundle');
  }
  assert.ok(manual.effects.every(e => e.type === 'copy'),'no automatic paste/send capability');
});

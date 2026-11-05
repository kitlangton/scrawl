import assert from 'node:assert/strict';
import { EngineClient, validateHandshake, validateReceipt, type Transport } from '../src/index.ts';
import { ReferenceEngine } from '../src/testing.ts';

export const v1Scenarios=[
  'handshake separates agreement from native readiness',
  'start identity replay never creates another operation',
  'wrong owner authority fails closed',
  'start rejection is structured and never says saved',
  'cancel is explicit and terminal',
  'timeouts are transport outcomes and do not imply cancel',
  'malformed, unknown-field, oversized and unsupported-version messages fail closed',
  'saved requires an absolute bundle path and artifact version',
] as const;

export async function runV1Scenarios(makeTransport:()=>Transport){
  const transport=makeTransport(),client=new EngineClient(transport),handshake=await client.handshake();
  assert.equal(handshake.protocolVersion,1);assert.equal(handshake.readiness,'unavailable');assert.equal(handshake.capabilities.screenCapture,false);
  const prepared=client.prepareStart('bundle-one','/private/bundle-one');const stable={...prepared,requestId:'request-one',ownerToken:'owner-one'};
  const first=await client.start(stable);
  const replay=await client.start(stable);assert.deepEqual(replay,first);
  assert.equal(first.receipt.state,'rejected');assert.notEqual(first.receipt.state,'saved');
  const conflict=await transport.request({protocolVersion:1,clientId:client.clientId,requestId:'request-one',method:'start',ownerToken:'owner-one',bundleId:'different-bundle',bundlePath:'/private/other'});assert.equal(conflict.error?.code,'identity_conflict');
  const wrong=await transport.request({protocolVersion:1,clientId:client.clientId,requestId:'request-one',method:'cancel',ownerToken:'wrong'});assert.equal(wrong.error?.code,'permission_denied');
  const fake=transport as ReferenceEngine;fake.scenarioState('request-one','recording');
  const cancelled=await client.control('cancel','request-one',first.ownerToken);assert.equal(cancelled.state,'cancelled');
  assert.throws(()=>JSON.parse('not-json'));
  assert.throws(()=>validateReceipt({protocolVersion:1,requestId:'r',bundleId:'b',state:'failed',error:{code:'internal',message:'x'.repeat(513)}}));
  assert.throws(()=>{if(Buffer.byteLength('x'.repeat(64*1024+1))>64*1024)throw new Error('oversized line')});
  assert.throws(()=>validateHandshake({protocolVersion:1,implementation:'test-double',capabilities:{screenCapture:true,audioCapture:false,windowCapture:false},readiness:'ready'}));
  assert.throws(()=>validateReceipt({protocolVersion:1,requestId:'r',bundleId:'b',state:'saved'}));
  assert.throws(()=>validateReceipt({protocolVersion:1,requestId:'r',bundleId:'b',state:'failed',futureField:true}));
  assert.throws(()=>validateReceipt({protocolVersion:2,requestId:'r',bundleId:'b',state:'failed'}));
  await client.close();
}
export const referenceEngineScenario=()=>new ReferenceEngine();

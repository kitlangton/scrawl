import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { TestContext } from 'node:test';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { ConnectSettings } from '../dist/settings-transport.cjs';

const snapshot={server:null,connected:false,uploadsEnabled:false,renewalRequired:false,
  reauthorizationRequired:false,accountInitial:null};
const account={version:1,snapshot,error:null,sharedLinksUrl:null,events:[]};
async function fixture(t:TestContext, respond:(request:Record<string,unknown>,socket:net.Socket)=>unknown = ()=>({account})) {
  const home=await fs.mkdtemp(path.join(os.tmpdir(),'connect-rpc-'));
  const clients=new Set<net.Socket>(), requests:Record<string,unknown>[]=[];
  let server:net.Server|undefined, client:ConnectSettings|undefined;
  t.after(async()=>{
    client?.close(); for (const socket of clients) socket.destroy();
    try { if(server?.listening)await new Promise<void>(resolve=>server!.close(()=>resolve())); }
    finally {await fs.rm(home,{recursive:true,force:true});}
  });
  const directory=path.join(home,'.config/apparent/agent-control');
  await fs.mkdir(directory,{recursive:true,mode:0o700}); await fs.chmod(directory,0o700);
  server=net.createServer(socket=>{
    clients.add(socket);socket.on('error',()=>{});socket.on('close',()=>clients.delete(socket));
    let bytes='';
    socket.on('data',chunk=>{
      bytes+=chunk.toString();
      while(bytes.includes('\n')) {
        const end=bytes.indexOf('\n'), request=JSON.parse(bytes.slice(0,end));bytes=bytes.slice(end+1);
        requests.push(request);const response=respond(request,socket);
        if(response!==undefined)socket.write(JSON.stringify(response)+'\n');
      }
    });
  });
  const address=path.join(directory,'capture.sock');
  await new Promise<void>(resolve=>server!.listen(address,()=>resolve()));await fs.chmod(address,0o600);
  client=new ConnectSettings(home);
  return {client,clients,requests,address};
}
async function until(check:()=>boolean) {
  const end=performance.now()+1000;
  while(!check()){assert.ok(performance.now()<end,'bounded fixture wait');await new Promise(resolve=>setTimeout(resolve,5));}
}

test('new transport projects public facts and an allow without consent sends no backend command',async t=>{
  const f=await fixture(t,()=>({account:{...account,token:'synthetic-secret',snapshot:{...snapshot,token:'synthetic-secret'}}}));
  const first=await f.client.request() as {status:string};
  assert.equal(first.status,'connected');assert.equal(JSON.stringify(first).includes('synthetic-secret'),false);
  await f.client.request('allow');
  assert.equal(f.requests.some(r=>r.command!==undefined),false);
  assert.equal(f.requests[0]!.connect,'backend');
});

for(const patch of [{version:2},{events:[{kind:'approval',operation:1,approval:{verificationUri:'file:///private',userCode:'CODE'}}]},
  {snapshot:{...snapshot,accountInitial:'private@example.test'}},{error:'private remote error'},{events:new Array(65).fill({})}]) {
  test('malformed account response closes instead of falling back '+JSON.stringify(patch).slice(0,90),async t=>{
    const f=await fixture(t,()=>({account:{...account,...patch}}));
    assert.deepEqual(await f.client.request(),{status:'error'});
    await until(()=>f.clients.size===0);
    assert.equal(f.requests.length,1);assert.equal(f.requests[0]!.connect,'backend');
  });
}

test('close invalidates queued opening work before filesystem admission can create an orphan lease',async t=>{
  const f=await fixture(t);
  const pending=f.client.request();f.client.close();
  assert.deepEqual(await pending,{status:'cancelled'});
  await new Promise(resolve=>setTimeout(resolve,20));
  assert.equal(f.requests.length,0);assert.equal(f.clients.size,0);
});

test('only an unsupported read-only opening can fall back to a legacy controller',async t=>{
  const view={title:'Connect',status:'Local',detail:'Local',note:'No uploads',server:null,
    verificationUri:null,userCode:null,buttons:[{command:'connect',title:'Connect',enabled:true}]};
  const f=await fixture(t,request=>request.connect==='backend'?{error:'invalid_request'}:{connect:view});
  assert.equal((await f.client.request() as {status:string}).status,'connected');
  await f.client.request('connect');
  assert.deepEqual(f.requests.map(r=>r.connect),['backend','settings','command']);
  assert.equal(f.requests[2]!.command,'connect');
});

test('idle EOF starts a fresh read lease without retrying an action or retaining old operation IDs',async t=>{
  const f=await fixture(t);
  await f.client.request();
  for(const socket of f.clients)socket.end();
  await until(()=>f.clients.size===0);
  assert.equal((await f.client.request() as {status:string}).status,'connected');
  assert.deepEqual(f.requests.map(r=>r.connect),['backend','backend']);
});

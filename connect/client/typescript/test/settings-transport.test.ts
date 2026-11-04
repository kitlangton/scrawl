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
  const end=performance.now()+3000;
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

function deviceAccount(snapshotValue:Record<string,unknown>=snapshot,events:unknown[]=[],extra:Record<string,unknown>={}) {
  return {version:1,snapshot:snapshotValue,error:null,sharedLinksUrl:null,events,deviceFlow:1,...extra};
}
const deviceStart={deviceCode:'private-device-secret',userCode:'ABCD-1234',
  verificationUri:'https://accounts.example/device?code=ABCD-1234',expiresIn:300,interval:1};
const deviceGrant={status:'approved',scope:'upload',accessToken:'private-access-secret',
  refreshToken:'private-refresh-secret',grantId:'private-grant-id',expiresIn:300,refreshExpiresIn:3600,
  unknownSecret:'private-unprojected-secret'};

test('capability device login requires consent, consumes queued receipts, and activates once without exposing replies',async t=>{
  let state:Record<string,unknown>={...snapshot,server:'https://accounts.example'},operation=41,deviceStep=0,activateCount=0;
  const queued:unknown[][]=[];
  const f=await fixture(t,(request)=>{
    if(request.connect==='backend')return {account:deviceAccount(state)};
    if(request.connect==='backend-read')return {account:deviceAccount(state,queued.shift()??[])};
    if(request.connect!=='backend-command')return {account:deviceAccount(state)};
    if(request.command==='device-login') {
      queued.push([{operation,kind:'device-start',reply:deviceStart,error:null}]);
      return {account:deviceAccount(state,[],{operation})};
    }
    if(request.command==='device-poll') {
      deviceStep++;
      assert.equal(request.deviceCode,'private-device-secret');
      assert.ok(typeof request.timeoutMs==='number'&&request.timeoutMs>0&&request.timeoutMs<=30_000);
      queued.push([{operation,kind:'device-poll',reply:deviceGrant,error:null}]);
      return {account:deviceAccount(state,[],{operation})};
    }
    if(request.command==='device-commit') {
      assert.equal(request.site,'https://accounts.example');
      state={server:'https://accounts.example',connected:true,uploadsEnabled:true,renewalRequired:false,
        reauthorizationRequired:false,accountInitial:null};
      queued.push([{operation,kind:'device-commit',reply:{committed:true,token:'private-commit-secret'},error:null}]);
      return {account:deviceAccount(state,[],{operation})};
    }
    if(request.command==='activate') { activateCount++; return {account:deviceAccount(state,[],{activated:true})}; }
    return {account:deviceAccount(state,[],{operation})};
  });

  const opening=await f.client.request() as Record<string,unknown>;
  assert.equal(opening.status,'connected');
  await f.client.request('connect');
  const beforeAllow=f.requests.length;
  assert.equal(f.requests.some(r=>r.command==='device-login'),false);
  await f.client.request('allow');
  await until(()=>f.requests.slice(beforeAllow).some(r=>r.command==='device-login'));
  assert.equal(f.requests.slice(beforeAllow).some(r=>r.command==='device-login'),true);
  assert.equal(f.requests.some(r=>r.command==='login'),false);
  await until(()=>queued.length>0);
  await f.client.request(); // Deliver device-start receipt.
  await until(()=>f.requests.some(r=>r.command==='device-poll'));
  await until(()=>queued.length>0);
  await f.client.request(); // Deliver approved poll receipt.
  await until(()=>f.requests.some(r=>r.command==='device-commit'));
  await until(()=>queued.length>0);
  const completed=await f.client.request() as Record<string,unknown>; // Commit receipt triggers exactly one activation.
  await f.client.request(); // The controller consumes its asynchronous login completion on the next read.
  await until(()=>activateCount===1);
  assert.equal(deviceStep,1);assert.equal(activateCount,1);
  assert.equal(f.requests.filter(r=>r.command==='activate').length,1);
  const observed=JSON.stringify([opening,completed]);
  for(const secret of ['private-device-secret','private-access-secret','private-refresh-secret',
    'private-grant-id','private-unprojected-secret','private-commit-secret']) assert.equal(observed.includes(secret),false,secret);
});

test('invalid approved grant does not commit or activate, and unsupported device capability fails closed',async t=>{
  let state:Record<string,unknown>={...snapshot,server:'https://accounts.example'},operation=73,activateCount=0,commitCount=0;
  const queued:unknown[][]=[];
  const f=await fixture(t,request=>{
    if(request.connect==='backend')return {account:deviceAccount(state)};
    if(request.connect==='backend-read')return {account:deviceAccount(state,queued.shift()??[])};
    if(request.connect==='backend-command'&&request.command==='device-login') {
      queued.push([{operation,kind:'device-start',reply:deviceStart,error:null}]);
      return {account:deviceAccount(state,[],{operation})};
    }
    if(request.connect==='backend-command'&&request.command==='device-poll') {
      queued.push([{operation,kind:'device-poll',reply:{...deviceGrant,grantId:'bad\nprivate-secret'},error:null}]);
      return {account:deviceAccount(state,[],{operation})};
    }
    if(request.connect==='backend-command'&&request.command==='device-commit') {commitCount++;return {account:deviceAccount(state,[],{operation})};}
    if(request.connect==='backend-command'&&request.command==='activate') {activateCount++;return {account:deviceAccount(state,[],{activated:true})};}
    return {account:deviceAccount(state,[],{operation})};
  });
  await f.client.request(); await f.client.request('connect'); await f.client.request('allow');
  await until(()=>queued.length>0); await f.client.request();
  await until(()=>f.requests.some(r=>r.command==='device-poll'));
  await until(()=>queued.length>0); await f.client.request();
  await new Promise(resolve=>setTimeout(resolve,20));
  await f.client.request(); // A normal renderer read drains the failed login receipt.
  assert.equal(commitCount,0);assert.equal(activateCount,0);
  assert.equal(f.requests.some(r=>r.command==='login'),false);

  f.client.close();
  const invalid=await fixture(t,()=>({account:{...deviceAccount(),deviceFlow:2}}));
  assert.deepEqual(await invalid.client.request(),{status:'error'});
  await until(()=>invalid.clients.size===0);
  assert.equal(invalid.requests.length,1);
  assert.equal(invalid.requests[0]!.connect,'backend');
});

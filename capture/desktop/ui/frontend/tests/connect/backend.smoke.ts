import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron as electron, expect } from '@playwright/test';

test('shipped account controller owns consent, activation, confirmation and stale browser cancellation', {timeout:45000}, async t => {
  const root = fileURLToPath(new URL('../..', import.meta.url));
  const home = await fs.mkdtemp(path.join(os.tmpdir(),'connect-ts-'));
  const clients = new Set<net.Socket>();
  let application: Awaited<ReturnType<typeof electron.launch>> | undefined;
  let server: net.Server | undefined;
  t.after(async()=>{
    try { await application?.close(); }
    finally {
      for (const client of clients) client.destroy();
      try { if (server?.listening) await new Promise<void>(resolve=>server!.close(()=>resolve())); }
      finally { await fs.rm(home,{recursive:true,force:true}); }
    }
  });
  const directory=path.join(home,'.config/apparent/agent-control');
  await fs.mkdir(directory,{recursive:true,mode:0o700}); await fs.chmod(directory,0o700);
  const snapshot={server:'https://connect.test',connected:false,uploadsEnabled:false,
    renewalRequired:false,reauthorizationRequired:false,accountInitial:null as string|null};
  const facts=()=>({cloud:{server:snapshot.server,enabled:snapshot.uploadsEnabled,uploadConsent:snapshot.uploadsEnabled},
    hostedSync:true,revocationPending:false,read:null,sharedLinksUrl:null,
    upload:snapshot.connected ? {server:snapshot.server,scope:'upload',authRequired:false,hasRefresh:false,
      expiresAt:null,refreshExpiresAt:null,accountInitial:snapshot.accountInitial,token:'synthetic-private-secret'} : null});
  let serial=0, login:number|undefined, activations=0, closed=0;
  let approved=false;
  let holdRead=false, held:(()=>void)|undefined;
  const events: Record<string,unknown>[]=[];
  const commands: {command:string;operation?:number}[]=[];
  server=net.createServer(socket=>{
    clients.add(socket); socket.on('error',()=>{});
    let buffer='', owned=false;
    socket.on('close',()=>{
      clients.delete(socket);
      if (owned) { closed+=1; if (login !== undefined) {snapshot.uploadsEnabled=false;login=undefined;} }
    });
    socket.on('data',chunk=>{
      buffer+=chunk.toString();
      while (buffer.includes('\n')) {
        const end=buffer.indexOf('\n'), line=buffer.slice(0,end); buffer=buffer.slice(end+1);
        if (line==='state') {socket.end('{"state":{"phase":"idle"}}\n');continue;}
        const request=JSON.parse(line);
        if (!['backend','backend-read','backend-command'].includes(request.connect)) {socket.end('{"error":"feature_disabled"}\n');continue;}
        owned=true;
        const account: Record<string,unknown>={version:1,snapshot:{...snapshot,credential:'synthetic-private-secret'},
          error:null,deviceFlow:1,stateFacts:1,facts:facts(),sharedLinksUrl:null,events:events.splice(0),credential:'synthetic-private-secret'};
        if (request.command) {
          commands.push({command:request.command,operation:request.operation});
          if (request.command==='device-login') {
            login=++serial; account.operation=login;
            events.push({operation:login,kind:'device-start',error:null,reply:{deviceCode:'synthetic-private-secret',
              verificationUri:'https://connect.test/device?code=TEST-CODE',userCode:'TEST-CODE',expiresIn:60,interval:1}});
          } else if (request.command==='device-poll') {
            assert.equal(request.operation,login); assert.equal(request.deviceCode,'synthetic-private-secret');
            account.operation=login;
            events.push({operation:login,kind:'device-poll',error:null,reply:approved ? {
              status:'approved',scope:'upload',accessToken:'synthetic-private-secret',refreshToken:'synthetic-private-secret',
              grantId:'synthetic-grant',expiresIn:300,refreshExpiresIn:3600,
            } : {status:'pending'}});
          } else if (request.command==='device-commit') {
            assert.equal(request.operation,login); assert.equal(request.site,'https://connect.test');
            account.operation=login;
            Object.assign(snapshot,{connected:true,uploadsEnabled:true,accountInitial:'M'});
            events.push({operation:login,kind:'device-commit',error:null,reply:null});
          } else if (request.command==='activate') {
            const valid=request.operation===login && snapshot.uploadsEnabled;
            account.activated=valid;
            if (valid) {activations+=1;login=undefined;}
          } else if (request.command==='disable' || request.command==='disconnect') {
            const operation=++serial; account.operation=operation; snapshot.uploadsEnabled=false;
            if (request.command==='disconnect') snapshot.connected=false;
            login=undefined;
            events.push({operation,kind:request.command,error:null});
          } else if (request.command==='cancel') login=undefined;
          else if (request.command==='probe') {
            const operation=++serial; account.operation=operation;
            snapshot.renewalRequired=false;
            events.push({operation,kind:'probe',health:'online'});
          }
        }
        account.snapshot={...snapshot,connected:!snapshot.connected,uploadsEnabled:!snapshot.uploadsEnabled,credential:'synthetic-private-secret'};
        account.facts=facts(); // The current renderer must use facts rather than this contradictory legacy snapshot.
        const reply=()=>socket.write(JSON.stringify({account})+'\n');
        if (holdRead && request.connect==='backend-read') {held=reply;continue;}
        reply();
      }
    });
  });
  const address=path.join(directory,'capture.sock');
  await new Promise<void>(resolve=>server!.listen(address,()=>resolve())); await fs.chmod(address,0o600);
  application=await electron.launch({args:[root,'--settings-connect'],env:{...process.env,HOME:home,XDG_CONFIG_HOME:path.join(home,'config')}});
  await application.evaluate(({shell})=>{ (global as any).opened=[];shell.openExternal=async uri=>{(global as any).opened.push(uri);}; });
  const page=await application.firstWindow();
  const card=page.getByRole('region',{name:'Apparent Connect'});
  await card.getByRole('button',{name:'Connect',exact:true}).click();
  await expect(card.getByText('Allow new capture uploads?',{exact:true})).toBeVisible();
  assert.equal(commands.length,0,'local consent presentation must not start backend operations');
  await card.getByRole('button',{name:'Allow & continue',exact:true}).click();
  await expect(card.getByRole('button',{name:'Open browser',exact:true})).toBeVisible();
  assert.ok(commands.some(c=>c.command==='device-login'));
  assert.equal(commands.some(c=>c.command==='login'),false);
  assert.equal(await page.getByText('synthetic-private-secret').count(),0);
  const projected=await page.evaluate(()=>window.apparent!.readConnectSettings());
  assert.equal(JSON.stringify(projected).includes('synthetic-private-secret'),false);
  assert.equal(activations,0);
  await card.getByRole('button',{name:'Open browser',exact:true}).click();
  await expect.poll(()=>application!.evaluate(()=>(global as any).opened.length)).toBe(1);
  approved=true;
  await expect(card.getByText('You’re connected.',{exact:true})).toBeVisible();
  assert.equal(activations,1);
  await card.getByRole('button',{name:'Account and connection options'}).click();
  await card.getByRole('button',{name:'Turn off uploads',exact:true}).click();
  await expect(card.getByText('Turn uploads off?',{exact:true})).toBeVisible();
  assert.equal(commands.filter(c=>c.command==='disable').length,0);
  await card.getByRole('button',{name:'Turn off uploads',exact:true}).click();
  await expect(card.getByText('Uploads are off.',{exact:true})).toBeVisible();
  assert.equal(commands.filter(c=>c.command==='disable').length,1);
  approved=false;
  await card.getByRole('button',{name:'Enable uploads',exact:true}).click();
  await card.getByRole('button',{name:'Allow & continue',exact:true}).click();
  await expect(card.getByRole('button',{name:'Open browser',exact:true})).toBeVisible();
  holdRead=true;
  await application.evaluate(({ipcMain,BrowserWindow})=>{
    const sender=BrowserWindow.getAllWindows()[0]!.webContents;
    (global as any).oldBrowser=(ipcMain as any)._invokeHandlers.get('apparent:connect-open-browser')({sender,senderFrame:sender.mainFrame});
  });
  await expect.poll(()=>typeof held).toBe('function');
  await page.reload(); holdRead=false;
  assert.deepEqual(await application.evaluate(()=>(global as any).oldBrowser),{status:'cancelled'});
  await expect.poll(()=>closed).toBe(1);
  held!();
  await expect(card.getByRole('button',{name:'Enable uploads',exact:true})).toBeVisible();
  assert.deepEqual(await application.evaluate(()=>(global as any).opened),['https://connect.test/device?code=TEST-CODE']);
  assert.equal(activations,1,'reload must not activate a pending login');
});

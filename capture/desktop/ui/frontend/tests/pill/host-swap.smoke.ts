import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {build} from 'esbuild';
import {_electron as electron, expect} from '@playwright/test';

test('replacement pill host resets revision and cannot inherit old callbacks or pending actions', async t => {
  const scratch = await mkdtemp(path.join(tmpdir(), 'record-host-swap-'));
  t.after(() => rm(scratch, {recursive:true, force:true}));
  await build({
    absWorkingDir: process.cwd(), bundle:true, format:'esm', platform:'browser', jsx:'automatic', preserveSymlinks:true,
    outfile:path.join(scratch,'renderer.js'),
    stdin:{resolveDir:process.cwd(), loader:'tsx', contents:`
      import React from 'react';
      import {createRoot} from 'react-dom/client';
      import {RecordingPill} from '@apparent/record/ui';
      const view=(revision,label)=>({revision,label,phase:'recording',clock:label,warning:null,notice:null,inputLevel:null,noteCount:0,reducedMotion:false,draggable:false,actions:[{id:'finish',label:'Finish',enabled:true,selected:false,primary:true}]});
      let oldSubscriber, resolveOldRead, rejectOldAction;
      let oldUnsubscribed=0;const calls=[];
      const oldView=view(100,'Old host');const newView=view(1,'New host');
      const common={manualDrag:false,layout:async()=>{},beginDrag:async()=>{},beginPaste:async()=>{},cancelPaste:async()=>{}};
      const oldHost={...common,read:()=>new Promise(resolve=>resolveOldRead=resolve),subscribe:callback=>{oldSubscriber=callback;callback(oldView);return()=>oldUnsubscribed++;},act:(revision)=>{calls.push(['old',revision]);return new Promise((_,reject)=>rejectOldAction=reject);}};
      const newHost={...common,read:async()=>newView,subscribe:callback=>{callback(newView);return()=>{};},act:async revision=>{calls.push(['new',revision]);}};
      const root=createRoot(document.getElementById('root'));root.render(<RecordingPill host={oldHost}/>);
      window.fixture={replace:()=>root.render(<RecordingPill host={newHost}/>),late:()=>{oldSubscriber(view(101,'Stale host'));resolveOldRead(oldView);rejectOldAction(new Error('late refusal'));},state:()=>({calls,oldUnsubscribed})};
    `},
  });
  await writeFile(path.join(scratch,'index.html'), '<div id="root"></div><script type="module" src="./renderer.js"></script>');
  await writeFile(path.join(scratch,'main.cjs'), `const {app,BrowserWindow}=require('electron');app.whenReady().then(()=>{const w=new BrowserWindow({show:false,webPreferences:{nodeIntegration:false,contextIsolation:true,sandbox:true}});w.loadFile(${JSON.stringify(path.join(scratch,'index.html'))});});`);
  const app = await electron.launch({args:[path.join(scratch,'main.cjs'),'--no-sandbox'],env:{...process.env,HOME:scratch,XDG_CONFIG_HOME:path.join(scratch,'config')}});
  t.after(() => app.close().catch(() => {}));
  const page=await app.firstWindow();
  await expect(page.getByText('Old host',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Finish',exact:true}).click();
  await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeDisabled();
  await page.evaluate(() => (window as any).fixture.replace());
  await expect(page.getByText('New host',{exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeEnabled();
  await page.evaluate(() => (window as any).fixture.late());
  await expect(page.getByText('New host',{exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeEnabled();
  await page.getByRole('button',{name:'Finish',exact:true}).click();
  assert.deepEqual(await page.evaluate(() => (window as any).fixture.state()), {calls:[['old',100],['new',1]],oldUnsubscribed:1});
});

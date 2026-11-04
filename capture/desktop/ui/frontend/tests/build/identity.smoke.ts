import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs/promises';
const require=createRequire(import.meta.url);
const {_electron:electron,expect}=require('@playwright/test');

test('running backend identity survives Electron launch, appearance and scrolling', {timeout:45000}, async t=>{
  const home=await fs.mkdtemp(path.join(os.tmpdir(),'build-ui-'));
  const expected=process.env.APPARENT_EXPECTED_BUILD_LABEL ?? '0.3.13 · abc1234 +modified';
  const application=await electron.launch({
    args:[path.resolve(import.meta.dirname,'../../electron/main.cjs'),'--no-sandbox'],
    cwd:home,
    env:{...process.env,HOME:home,
      APPARENT_RUNNING_BUILD_LABEL:process.env.APPARENT_EXPECTED_BUILD_LABEL ? process.env.APPARENT_RUNNING_BUILD_LABEL : expected},
  });
  t.after(async()=>{await application.close().catch(()=>{});await fs.rm(home,{recursive:true,force:true});});
  const page=await application.firstWindow();
  assert.equal(await page.evaluate(()=>window.apparentBuild.read()),expected);
  const identity=expected.split(' · ')[1] ?? 'build unknown';
  const marker=page.locator('.apparent-build-identity');
  await expect(marker).toHaveText(identity);
  for(const theme of ['light','dark']) {
    await application.evaluate(({nativeTheme}:any,theme:string)=>{nativeTheme.themeSource=theme;},theme);
    await page.locator('.apparent-window-content').evaluate((element:HTMLElement)=>element.scrollTo(0,element.scrollHeight));
    await expect(marker).toBeVisible();
    assert.equal(await marker.evaluate((element:HTMLElement)=>{
      const r=element.getBoundingClientRect();
      return r.top>=0 && r.left>=0 && r.bottom<=innerHeight && r.right<=innerWidth &&
        getComputedStyle(element).pointerEvents==='none' && element.tabIndex<0;
    }),true);
  }
});

test('notes build label passes through background clicks', {timeout:30000}, async t=>{
  const home=await fs.mkdtemp(path.join(os.tmpdir(),'build-notes-'));
  const application=await electron.launch({
    args:['--no-sandbox',path.resolve(import.meta.dirname,'../pill/host.cjs'),
      '--notes-fixture','--capture-frame=0,0,1280,800'],
    env:{...process.env,HOME:home},
  });
  t.after(async()=>{await application.close().catch(()=>{});await fs.rm(home,{recursive:true,force:true});});
  const messages:any[]=[];
  let pending='';
  application.process().stdout.on('data',(chunk:Buffer)=>{
    pending+=chunk.toString();
    const lines=pending.split('\n');pending=lines.pop()!;
    for(const line of lines){try{messages.push(JSON.parse(line));}catch{}}
  });
  await expect.poll(()=>application.context().pages().length).toBe(2);
  const page=application.context().pages().find((p:any)=>p.url().endsWith('#notes-popup'))!;
  await application.evaluate((_electron:any,state:any)=>process.stdin.emit('data',JSON.stringify(state)+'\n'),{
    mode:'editor',current:1,position:[600,280],notes:[],
    drafts:[{pin:1,t:123,x:.5,y:.35,text:'Keep this draft'}],error:null,
  });
  await expect(page.getByRole('textbox',{name:'Note text'})).toHaveValue('Keep this draft');
  const marker=page.locator('.apparent-build-identity');
  await expect(marker).toBeVisible();
  assert.equal(await marker.evaluate((element:HTMLElement)=>{
    const r=element.getBoundingClientRect();
    return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)===element.closest('main');
  }),true,'Build text and its reserved row must pass clicks to the existing notes background');
  const point=await marker.evaluate((element:HTMLElement)=>{
    const r=element.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};
  });
  await page.mouse.click(point.x,point.y);
  await expect.poll(()=>messages.some(message=>message.action==='dismiss')).toBe(true);
});

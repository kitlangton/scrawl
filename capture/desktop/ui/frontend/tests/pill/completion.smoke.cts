const {test}=require('node:test');
const assert=require('node:assert/strict');
const {_electron:electron,expect}=require('@playwright/test');
const {spawn}=require('node:child_process');
const {createInterface}=require('node:readline');
const {once}=require('node:events');
const path=require('node:path');
const fs=require('node:fs/promises');
const os=require('node:os');

// Real packaged Node runtime and real Electron clipboard/pill, with synthetic facts.
// This does not substitute for the native finalizer + installed capture acceptance.
test('packaged Record result reaches the pill and survives renderer reload',async t=>{
 const home=await fs.mkdtemp(path.join(os.tmpdir(),'record-pill-'));
 const application=await electron.launch({args:[path.resolve(__dirname,'host.cjs'),'--no-sandbox'],
  env:{...process.env,HOME:home,XDG_CONFIG_HOME:path.join(home,'config')}});
 t.after(async()=>{await application.close().catch(()=>{});await fs.rm(home,{recursive:true,force:true});});
 const page=await application.firstWindow();
 const runtime=require('electron');
 const worker=spawn(runtime,[path.resolve(__dirname,'../../electron/record-completion/worker.js')],
  {env:{...process.env,ELECTRON_RUN_AS_NODE:'1'},stdio:['pipe','pipe','pipe']});
 t.after(()=>worker.kill());
 const exit=once(worker,'exit');
 const lines=createInterface({input:worker.stdout})[Symbol.asyncIterator]();
 worker.stdin.write(JSON.stringify({version:1,current:true,
  context:{capture:{generation:'9',captureId:'synthetic-a',requestId:'synthetic-request'},platform:process.platform==='linux'?'linux':'macos',
   source:'screen',agentRequested:false,automaticTrialEnabled:false,lab:false,hyprland:false,wayland:false},
  outcome:{kind:'saved',bundlePath:'/synthetic/a.apparent'}})+'\n');
 const initial=JSON.parse((await lines.next()).value);
 assert.equal(initial.effects[0].mode,'bundle');
 // This provider fixture writes and reads an isolated native clipboard before ACK.
 const text='Synthetic saved capture handoff';
 assert.equal(await application.evaluate(({clipboard},text)=>{clipboard.writeText(text);return clipboard.readText();},text),text);
 worker.stdin.write(JSON.stringify({result:'copied'})+'\n');
 const completed=JSON.parse((await lines.next()).value);
 assert.deepEqual(await exit,[0,null]);
 assert.equal(completed.state.observation.clipboard,'copied');
 const view={revision:1,phase:'completed',clock:'00:04',label:completed.state.status,warning:null,notice:null,
  reducedMotion:true,draggable:false,inputLevel:null,actions:[
   {id:'paste',label:'Paste',enabled:true,selected:false,primary:true},
   {id:'inspect',label:'Inspect',enabled:true,selected:false,primary:false},
   {id:'dismiss',label:'Dismiss',enabled:true,selected:false,primary:false}]};
 await application.evaluate((_electron,view)=>{process.stdin.emit('data',JSON.stringify({type:'state',view})+'\n');
  process.stdin.emit('data',JSON.stringify({type:'activate'})+'\n');},view);
 await expect(page.getByText('Saved · copied',{exact:true})).toBeVisible();
 await page.reload();
 await expect(page.getByText('Saved · copied',{exact:true})).toBeVisible();
 assert.equal(await application.evaluate(({clipboard})=>clipboard.readText()),text);
 await expect(page.getByRole('button',{name:'Inspect',exact:true})).toBeEnabled();
 assert.equal(await page.evaluate(()=>document.body.textContent.includes('/synthetic/')),false);
});

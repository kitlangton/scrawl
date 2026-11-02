const {test}=require('node:test');
const assert=require('node:assert/strict');
const { _electron:electron,expect}=require('@playwright/test');
const path=require('node:path');
const fs=require('node:fs/promises');
const os=require('node:os');
const base={revision:1,phase:'recording',clock:'00:12',label:'Recording',warning:null,reducedMotion:false,draggable:true,
  actions:[{id:'whiteboard',label:'Whiteboard',enabled:true,selected:false,primary:false},
    {id:'finish',label:'Finish',enabled:true,selected:false,primary:true},
    {id:'cancel',label:'Cancel',enabled:true,selected:false,primary:false}]};
test('shared pill renders authoritative states and isolates commands',async t=>{
 const home=await fs.mkdtemp(path.join(os.tmpdir(),'pill-'));
 const application=await electron.launch({args:[path.resolve(__dirname,'host.cjs'),'--no-sandbox'],
  env:{...process.env,HOME:home,XDG_CONFIG_HOME:path.join(home,'config')}});
 t.after(async()=>{await application.close().catch(()=>{});await fs.rm(home,{recursive:true,force:true});});
 const page=await application.firstWindow();
 const evidence=process.env.APPARENT_PILL_EVIDENCE;
 if(evidence)await fs.mkdir(evidence,{recursive:true,mode:0o700});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 const sent=[];let stream='';application.process().stdout.on('data',data=>{stream+=data.toString();let end;while((end=stream.indexOf('\n'))>=0){const line=stream.slice(0,end);stream=stream.slice(end+1);try{sent.push(JSON.parse(line));}catch{}}});
 async function publish(state){await application.evaluate((_electron,state)=>{process.stdin.emit('data',JSON.stringify({type:'state',view:state})+'\n');},state);}
 await publish(base);
 await application.evaluate(()=>process.stdin.emit('data',JSON.stringify({type:'activate'})+'\n'));
 await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeVisible();
 await expect.poll(()=>sent.some(message=>message.ready===true)).toBe(true);
 const nativeId=await application.evaluate(({BrowserWindow})=>Number(BrowserWindow.getAllWindows()[0].getMediaSourceId().split(':')[1]));
 assert.equal(sent.find(message=>message.ready===true).windowId,nativeId);
 assert.equal(await page.evaluate(()=>typeof window.require),'undefined');
 assert.equal(await page.evaluate(()=>typeof window.process),'undefined');
 assert.deepEqual(await page.evaluate(()=>Object.keys(window.apparentPill)),['read','layout','beginPaste','cancelPaste','act','subscribe']);
 if(evidence)await page.screenshot({path:path.join(evidence,'recording.png')});
 await page.getByRole('button',{name:'Finish',exact:true}).click();
 await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeDisabled();
 await expect.poll(()=>sent.filter(x=>x.action==='finish').length).toBe(1);
 assert.deepEqual(sent.find(x=>x.action==='finish'),{revision:1,action:'finish'});
 assert.equal(await page.evaluate(async()=>{try{await window.apparentPill.act(1,'paste');return false;}catch{return true;}}),true);
 await publish({...base,revision:2,phase:'processing',label:'Processing recording…',draggable:false,actions:[]});
 await expect(page.getByText('Processing recording…',{exact:true})).toBeVisible();
 assert.equal(await page.getByRole('button').count(),0);
 await publish({...base,revision:3,warning:'Microphone input stopped; screen recording continues',actions:base.actions.map(a=>({...a,primary:false}))});
 await expect(page.getByRole('status')).toContainText('screen recording continues');
 await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeEnabled();
 await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeInViewport();
 await expect.poll(()=>page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth && document.documentElement.scrollHeight<=innerHeight)).toBe(true);
 await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
 const dark=await page.locator('main').evaluate(el=>getComputedStyle(el).backgroundColor);
 await page.emulateMedia({colorScheme:'light'});
 assert.notEqual(await page.locator('main').evaluate(el=>getComputedStyle(el).backgroundColor),dark);
 assert.equal(await page.locator('.indicator').evaluate(el=>getComputedStyle(el).animationName),'none');
 if(evidence)await page.screenshot({path:path.join(evidence,'warning-light.png')});
 await publish({...base,revision:4,phase:'selecting',label:'Choose what to record',draggable:false,actions:[
  {id:'select-area',label:'Area',enabled:true,selected:false,primary:false},
  {id:'select-screen',label:'Screen',enabled:true,selected:true,primary:false},
  {id:'record',label:'Record',enabled:true,selected:false,primary:true},base.actions[2]]});
 await expect(page.getByRole('button',{name:'Screen',exact:true})).toHaveAttribute('aria-pressed','true');
 assert.equal(await page.getByRole('button',{name:'Window',exact:true}).count(),0);
 if(evidence)await page.screenshot({path:path.join(evidence,'selecting.png')});
 await page.keyboard.press('Escape');
 await expect.poll(()=>sent.filter(x=>x.action==='cancel').length).toBe(1);
 await publish({...base,revision:5,phase:'completed',label:'Saved',draggable:false,actions:[
  {id:'paste',label:'Paste',enabled:true,selected:false,primary:true},
  {id:'copy-link',label:'Copy link',enabled:false,selected:false,primary:false},
  {id:'dismiss',label:'Dismiss',enabled:true,selected:false,primary:false}]});
 await expect(page.getByRole('button',{name:'Copy link',exact:true})).toBeDisabled();
 if(evidence)await page.screenshot({path:path.join(evidence,'completed.png')});
 await page.reload();
 await expect(page.getByRole('button',{name:'Paste',exact:true})).toBeEnabled();
 assert.equal(await page.evaluate(async()=>{try{await fetch('https://example.com');return false;}catch{return true;}}),true);
 assert.equal(await page.evaluate(()=>window.open('https://example.com')),null);
 assert.equal(await application.evaluate(({BrowserWindow})=>BrowserWindow.getAllWindows().length),1);
 // Right-click must never invoke Paste; a cancelled gesture must not invoke it either.
 const before=sent.filter(x=>x.action==='paste').length;
 await page.getByRole('button',{name:'Paste',exact:true}).click({button:'right'});
 assert.equal(sent.filter(x=>x.action==='paste').length,before);
 assert.deepEqual(errors,[]);
});

test('failed renderer assets never report readiness and the parent can retain native controls',async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'pill-assets-'));
 await fs.mkdir(path.join(root,'electron'),{recursive:true});await fs.mkdir(path.join(root,'dist'));
 await fs.cp(path.resolve(__dirname,'../../electron/pill'),path.join(root,'electron/pill'),{recursive:true});
 await fs.copyFile(path.resolve(__dirname,'../../dist/pill.html'),path.join(root,'dist/pill.html'));
 // The production HTML references deliberately absent production chunks.
 await fs.writeFile(path.join(root,'host.cjs'),"Object.defineProperty(process,'stdin',{value:new(require('node:stream').PassThrough)()});require('./electron/pill/main.cjs');\n");
 const application=await electron.launch({args:[path.join(root,'host.cjs'),'--no-sandbox'],env:{...process.env,HOME:root,XDG_CONFIG_HOME:path.join(root,'config')}});
 t.after(async()=>{await application.close().catch(()=>{});await fs.rm(root,{recursive:true,force:true});});
 const output=[];application.process().stdout.on('data',data=>output.push(data.toString()));
 await application.firstWindow();
 const hidden=await application.evaluate(({BrowserWindow})=>!BrowserWindow.getAllWindows()[0].isVisible());
 assert.equal(hidden,true);
 const closed=application.waitForEvent('close',{timeout:8000});
 const heartbeat=setInterval(()=>{void application.evaluate((_e,state)=>process.stdin.emit('data',JSON.stringify({type:'state',view:state})+'\n'),base).catch(()=>{});},200);
 try{await closed;}finally{clearInterval(heartbeat);}
 assert.equal(output.join('').includes('"ready":true'),false);
});

test('startup placement preserves a fragmented next parent frame',async t=>{
 const home=await fs.mkdtemp(path.join(os.tmpdir(),'pill-framing-'));
 const application=await electron.launch({args:[path.join(__dirname,'framing.cjs'),'--no-sandbox'],env:{...process.env,HOME:home,XDG_CONFIG_HOME:path.join(home,'config')}});
 t.after(async()=>{await application.close().catch(()=>{});await fs.rm(home,{recursive:true,force:true});});
 const page=await application.firstWindow();
 await expect(page.getByRole('button',{name:'Finish',exact:true})).toBeVisible();
 assert.equal((await page.evaluate(()=>window.apparentPill.read())).revision,1);
});

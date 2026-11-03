const {app,BrowserWindow,ipcMain,session,nativeTheme}=require('electron');
const {pathToFileURL}=require('node:url');
const path=require('node:path'),fs=require('node:fs'),readline=require('node:readline');
const profile=fs.mkdtempSync(path.join(require('node:os').tmpdir(),'apparent-share-'));
app.setPath('userData',profile);app.setName('Apparent');nativeTheme.themeSource='system';
app.on('quit',()=>{try{fs.rmSync(profile,{recursive:true,force:true});}catch{}});
let latest,window;
let deliverySent=false;
const deliver=()=>{if(latest?.state==='prepared'&&window&&!window.isDestroyed()&&window.isFocused()&&!deliverySent){deliverySent=true;process.stdout.write('deliver\n');}};
const states=new Set(['confirm','preparing','prepared','copied','failed','cancelled','expired']);
const lines=readline.createInterface({input:process.stdin});
lines.on('line',line=>{try{
 if(Buffer.byteLength(line)>4096)throw Error();
 const state=JSON.parse(line);
 if(Object.keys(state).length!==2||typeof state.capture!=='string'||state.capture.length>1024||!states.has(state.state))throw Error();
 latest=state;if(window&&!window.isDestroyed())window.webContents.send('share:state',latest);deliver();
}catch{app.quit();}});
lines.on('close',()=>app.quit());
app.whenReady().then(()=>{
 session.defaultSession.setPermissionRequestHandler((_w,_p,reply)=>reply(false));
 session.defaultSession.setPermissionCheckHandler(()=>false);
 const page=pathToFileURL(path.join(__dirname,'../../dist/index.html')).href+'#share';
 window=new BrowserWindow({width:520,height:480,resizable:false,title:'Share capture',show:false,webPreferences:{preload:path.join(__dirname,'preload.cjs'),contextIsolation:true,nodeIntegration:false,sandbox:true,webSecurity:true,spellcheck:false}});
 window.removeMenu();window.webContents.setWindowOpenHandler(()=>({action:'deny'}));
 window.webContents.on('will-navigate',event=>event.preventDefault());
 window.webContents.on('will-attach-webview',event=>event.preventDefault());
 window.webContents.on('render-process-gone',()=>app.quit());
 window.webContents.on('before-input-event',(_event,input)=>{if(input.type==='keyDown'&&input.key==='Escape')app.quit();});
 const validate=(event,args)=>{if(event.sender!==window.webContents||event.senderFrame!==window.webContents.mainFrame||event.senderFrame.url!==page||args.length)throw Error('Forbidden sharing request');};
 ipcMain.handle('share:ready',(event,...args)=>{validate(event,args);return latest;});
 let admitted=false;
 for(const action of ['confirm','cancel'])ipcMain.handle('share:'+action,(event,...args)=>{
  validate(event,args);
  if(action==='confirm'){if(admitted||latest?.state!=='confirm')throw Error('Invalid sharing state');admitted=true;}
  process.stdout.write(action+'\n');if(action==='cancel')app.quit();
 });
 window.on('closed',()=>app.quit());window.on('focus',deliver);
 window.webContents.once('did-finish-load',()=>{window.show();window.focus();if(latest)window.webContents.send('share:state',latest);});
 void window.loadURL(page);
});

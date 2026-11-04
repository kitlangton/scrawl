require('../runtime.cjs').installBuildIdentity(require('electron'));
require('../runtime.cjs').installExceptionReporting(require('electron'));
const {app,BrowserWindow,ipcMain,session,nativeTheme}:typeof import('electron')=require('electron');
const {pathToFileURL}:typeof import('node:url')=require('node:url');
const path:typeof import('node:path')=require('node:path');
const fs:typeof import('node:fs')=require('node:fs');
const readline:typeof import('node:readline')=require('node:readline');
const profile=fs.mkdtempSync(path.join(require('node:os').tmpdir(),'apparent-share-'));
app.setPath('userData',profile);app.setName('Apparent');nativeTheme.themeSource='system';
app.on('quit',()=>{try{fs.rmSync(profile,{recursive:true,force:true});}catch{}});
type Snapshot={capture:string;state:string};
let latest:Snapshot|undefined,window:import('electron').BrowserWindow|undefined;
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
 const ownedWindow=new BrowserWindow({width:520,height:480,resizable:false,title:'Share capture',show:false,webPreferences:{preload:path.join(__dirname,'preload.cjs'),contextIsolation:true,nodeIntegration:false,sandbox:true,webSecurity:true,spellcheck:false}});
 window=ownedWindow;
 ownedWindow.removeMenu();ownedWindow.webContents.setWindowOpenHandler(()=>({action:'deny'}));
 ownedWindow.webContents.on('will-navigate',event=>event.preventDefault());
 ownedWindow.webContents.on('will-attach-webview',event=>event.preventDefault());
 ownedWindow.webContents.on('render-process-gone',()=>app.quit());
 ownedWindow.webContents.on('before-input-event',(_event,input)=>{if(input.type==='keyDown'&&input.key==='Escape')app.quit();});
 const validate=(event:import('electron').IpcMainInvokeEvent,args:unknown[])=>{if(event.sender!==ownedWindow.webContents||event.senderFrame!==ownedWindow.webContents.mainFrame||event.senderFrame.url!==page||args.length)throw Error('Forbidden sharing request');};
 ipcMain.handle('share:ready',(event,...args)=>{validate(event,args);return latest;});
 let admitted=false;
 for(const action of ['confirm','cancel'])ipcMain.handle('share:'+action,(event,...args)=>{
  validate(event,args);
  if(action==='confirm'){if(admitted||latest?.state!=='confirm')throw Error('Invalid sharing state');admitted=true;}
  process.stdout.write(action+'\n');if(action==='cancel')app.quit();
 });
 ownedWindow.on('closed',()=>app.quit());ownedWindow.on('focus',deliver);
 ownedWindow.webContents.once('did-finish-load',()=>{ownedWindow.show();ownedWindow.focus();if(latest)ownedWindow.webContents.send('share:state',latest);});
 void ownedWindow.loadURL(page);
});

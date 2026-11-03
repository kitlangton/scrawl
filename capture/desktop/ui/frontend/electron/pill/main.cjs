'use strict';
const {app,BrowserWindow,ipcMain,session,screen,nativeTheme,Menu} = require('electron');
const path=require('node:path');
const {pathToFileURL}=require('node:url');
const protocol=require('./protocol.cjs');
const geometry=require('./geometry.cjs');
if(!['linux','darwin'].includes(process.platform))process.exit(1);
app.setName('Apparent recording controls');
app.setPath('userData',path.join(app.getPath('appData'),'apparent-pill'));
nativeTheme.themeSource='system';
const page=pathToFileURL(path.join(__dirname,'../../dist/pill/index.html')).href;
let window,current=null,buffer='',pending=false,loaded=false,placed=false,activated=false,pendingPlace=null;
let footprint={rowY:0,rowHeight:58};
let lastInput=Date.now(),hovered=false,expected=null,rendererReady=false,readySent=false,renderDeadline=Date.now()+5000,pasteGesture=null,lastFrame='';
function position(bounds){
 const next={...window.getBounds(),...bounds};
 const area=screen.getDisplayMatching(next).workArea;
 expected={...next,...geometry.clamp(next,area)};window.setBounds(expected);
}
function applyPlacement(point){
  pendingPlace=point;
  if(!window)return;
  const physical={x:Math.round(point[0]),y:Math.round(point[1])};
  const anchor=process.platform==='linux'?screen.screenToDipPoint(physical):physical;
  const area=screen.getDisplayNearestPoint(anchor).workArea;
  const bounds=window.getBounds();
  position({x:Math.round(area.x+(area.width-bounds.width)/2),y:area.y+34});placed=true;
}
function emit(message){process.stdout.write(JSON.stringify(message)+'\n');}
function reportFrame(){
  if(process.platform!=='darwin' || !rendererReady || current?.phase!=='recording' || !window.isVisible())return;
  const b=window.getBounds(),frame=[b.x,b.y,b.width,b.height],revision=current.revision,key=JSON.stringify({revision,frame});
  if(key!==lastFrame){lastFrame=key;emit({revision,frame});}
}
function publish(value){
  const next=protocol.view(value);
  if(current && next.revision<current.revision)throw Error('Old pill state');
  if(!current || next.revision!==current.revision)pending=false;
  if(current && next.revision!==current.revision)pasteGesture=null;
  current=next;lastInput=Date.now();
  if(loaded)window.webContents.send('pill:state',current);
  visibility();
}
function visibility(){
  if(!window || window.isDestroyed())return;
  if(!activated || current?.phase==='hidden'){
    if(window.isVisible())window.hide();
    return;
  }
  if(rendererReady && current && !window.isVisible())window.showInactive();
}
function valid(event,args){
  if(!window || window.isDestroyed() || event.sender!==window.webContents ||
    event.senderFrame!==window.webContents.mainFrame || event.senderFrame.url!==page || args.length)throw Error('Forbidden pill bridge request');
}
process.stdin.setEncoding('utf8');
process.stdin.on('data',chunk=>{
  buffer+=chunk;
  if(Buffer.byteLength(buffer)>65536){app.exit(1);return;}
  let end;
  while((end=buffer.indexOf('\n'))>=0){
    const line=buffer.slice(0,end);buffer=buffer.slice(end+1);
    try{
      const message=JSON.parse(line);
      if(message.type==='state')publish(message.view);
      else if(message.type==='activate'){activated=true;if(loaded)visibility();emit({activated:true});}
      else if(message.type==='place' && Array.isArray(message.point) && message.point.length===2 && message.point.every(Number.isFinite)){
        applyPlacement(message.point);
      }else if(message.type==='position' && process.platform==='linux' && Array.isArray(message.point) && message.point.length===2 && message.point.every(Number.isFinite)){
        if(window && message.revision===current?.revision && current.phase==='recording' && current.draggable){
          const point=screen.screenToDipPoint({x:Math.round(message.point[0]),y:Math.round(message.point[1])});
          position(point);
        }
      }else throw Error('Invalid parent message');
    }catch{app.exit(1);}
  }
});
process.stdin.on('end',()=>app.quit());
process.stdin.on('error',()=>app.quit());
app.whenReady().then(()=>{
  Menu.setApplicationMenu(null);
  if(process.platform==='darwin')app.setActivationPolicy('accessory');
  session.defaultSession.setPermissionRequestHandler((_contents,_permission,reply)=>reply(false));
  session.defaultSession.setPermissionCheckHandler(()=>false);
  session.defaultSession.webRequest.onBeforeRequest((details,reply)=>reply({cancel:!details.url.startsWith('file:')}));
  window=new BrowserWindow({...(process.platform==='darwin'?{type:'panel',acceptFirstMouse:true}:{focusable:false}),width:640,height:58,show:false,frame:false,transparent:true,resizable:false,
    fullscreenable:false,skipTaskbar:true,hasShadow:false,alwaysOnTop:true,title:'Apparent recording controls',
    webPreferences:{preload:path.join(__dirname,'preload.cjs'),contextIsolation:true,sandbox:true,nodeIntegration:false,webSecurity:true,spellcheck:false}});
  if(pendingPlace)applyPlacement(pendingPlace);
  window.setAlwaysOnTop(true,'screen-saver');
  window.setVisibleOnAllWorkspaces(true,{visibleOnFullScreen:true});
  window.setContentProtection(false);
  window.webContents.setWindowOpenHandler(()=>({action:'deny'}));
  window.webContents.on('will-navigate',event=>event.preventDefault());
  window.webContents.on('will-attach-webview',event=>event.preventDefault());
  window.webContents.on('page-title-updated',event=>event.preventDefault());
  window.webContents.on('render-process-gone',()=>app.exit(1));
  window.webContents.on('did-start-loading',()=>{loaded=false;pending=false;rendererReady=false;pasteGesture=null;renderDeadline=Date.now()+5000;});
  ipcMain.handle('pill:read',(event,...args)=>{valid(event,args);return current;});
  ipcMain.handle('pill:act',(event,revision,action,drop=false,...args)=>{
    valid(event,args);
    if(pending)throw Error('Pill action pending');
    if(typeof drop!=='boolean' || drop && action!=='paste')throw Error('Invalid paste gesture');
    const message=protocol.command(current,revision,action);
    if(drop){
      if(!pasteGesture || pasteGesture.revision!==revision || Date.now()>pasteGesture.until)throw Error('Paste gesture expired');
      const dip=screen.getCursorScreenPoint();
      const point=process.platform==='linux'?screen.dipToScreenPoint(dip):dip;
      message.target=[point.x,point.y];
    }
    pasteGesture=null;
    pending=true;emit(message);
  });
  window.webContents.on('did-finish-load',()=>{
    loaded=true;
    if(!placed){const area=screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).workArea;
      position({x:Math.round(area.x+(area.width-640)/2),y:area.y+34});}
    if(current)window.webContents.send('pill:state',current);
    visibility();
  });
  // Renderer never chooses native dimensions or screen coordinates.
  // Parent's bounded state defines content; main measures its own trusted document.
  async function measure(){
    if(!loaded || window.isDestroyed())return;
    if(current?.phase!=='hidden' && Date.now()-lastInput>5000){app.exit(1);return;}
    try{
      const area=screen.getDisplayMatching(window.getBounds()).workArea;
      const size=await window.webContents.executeJavaScript(`(() => {document.documentElement.style.setProperty('--pill-limit', '${area.width}px');const element=document.querySelector('main'),rect=element?.getBoundingClientRect();return rect && {width:Math.ceil(rect.width),height:Math.ceil(rect.height),rowHeight:Math.ceil(document.querySelector('.pill-row')?.getBoundingClientRect().height??rect.height),revision:Number(element.dataset.revision)};})()`);
      if(!size || !Number.isFinite(size.width)||!Number.isFinite(size.height))return false;
      const bounds=window.getBounds();
      const width=Math.min(area.width,Math.max(100,Math.ceil(size.width)));
      const height=Math.min(area.height,360,Math.max(58,Math.ceil(size.height)));
      const rowHeight=Math.min(height,Math.max(56,Math.ceil(size.rowHeight)));
      const next=geometry.layout(bounds,area,width,height,rowHeight,footprint.rowY);
      await window.webContents.executeJavaScript(`document.querySelector('main')?.setAttribute('data-pocket-side', '${next.side}')`);
      if(size.revision!==current?.revision)return false;
      footprint=next;
      if(bounds.width!==width || bounds.height!==height || bounds.y!==next.bounds.y)position(next.bounds);
      if(process.platform==='linux')window.setShape(geometry.shape(width,height,next.rowY,rowHeight));
      return size.revision===current?.revision;
    }catch{app.exit(1);}
  }
  const sizing=setInterval(()=>{
    if(!rendererReady && Date.now()>renderDeadline){app.exit(1);return;}
    void measure();
    reportFrame();
  },250);
  ipcMain.handle('pill:layout',async(event,revision,...args)=>{
    valid(event,args);
    if(!Number.isSafeInteger(revision) || revision!==current?.revision)return;
    const committed=current.phase==='hidden' || await measure();
    if(committed && revision===current?.revision){
      // React invokes this after committing the authoritative snapshot. HTML load
      // alone must never retire the functioning previous presentation.
      rendererReady=true;
      if(!readySent){
        const windowId=Number(window.getMediaSourceId().match(/^window:(\d+):\d+$/)?.[1]);
        if(!Number.isSafeInteger(windowId) || windowId<=0 || windowId>0xffffffff){app.exit(1);return;}
        if(process.platform==='linux')window.setTitle('Apparent Recording Controls '+windowId.toString(16));
        readySent=true;emit({ready:true,windowId});
      }
      visibility();
    }
  });
  ipcMain.handle('pill:begin-paste',(event,revision,...args)=>{
    valid(event,args);protocol.command(current,revision,'paste');
    if(pending)throw Error('Pill action pending');
    pasteGesture={revision,until:Date.now()+5000};
    window.setIgnoreMouseEvents(false);
  });
  ipcMain.handle('pill:cancel-paste',(event,...args)=>{valid(event,args);pasteGesture=null;});
  ipcMain.handle('pill:begin-drag',async(event,revision,...args)=>{
    valid(event,args);
    if(process.platform!=='linux' || revision!==current?.revision || current.phase!=='recording' || !current.draggable || pending)throw Error('Drag unavailable');
    const point=screen.getCursorScreenPoint(),bounds=window.getBounds();
    const lead=await window.webContents.executeJavaScript(`(() => {const r=document.querySelector('.lead')?.getBoundingClientRect();return r && {x:r.x,y:r.y,width:r.width,height:r.height};})()`);
    if(revision!==current?.revision || !current.draggable || !lead || point.x<bounds.x+lead.x || point.y<bounds.y+lead.y || point.x>=bounds.x+lead.x+lead.width || point.y>=bounds.y+lead.y+lead.height)throw Error('Drag target changed');
    const physical=screen.dipToScreenPoint(point);
    emit({revision,action:'drag',target:[physical.x,physical.y]});
  });
  const pointerTimer=setInterval(()=>{
    if(!loaded || window.isDestroyed())return;
    const inside=window.isVisible() && geometry.contains(screen.getCursorScreenPoint(),window.getBounds(),footprint.rowY,footprint.rowHeight);
    if(inside!==hovered){hovered=inside;emit({hovered});}
    if(pasteGesture && Date.now()>pasteGesture.until)pasteGesture=null;
    if(process.platform==='darwin')window.setIgnoreMouseEvents(!inside && !pasteGesture,{forward:true});
  },16);
  window.on('closed',()=>{clearInterval(sizing);clearInterval(pointerTimer);app.quit();});
  // Hyprland assigns even override-redirect XWayland windows to a workspace.
  // Rust's existing native adapter pins only this exact child/window identity.
  if(process.platform==='linux'){
    for(const event of ['show','move','resize'])window.on(event,()=>{
      const visible=window.isVisible();
      // Initial measurement/placement also resize the hidden window. Native
      // admission begins only after the parent hands over its presentation.
      if(activated && rendererReady && current && current.phase!=='hidden' && visible)
        emit({pin:true,phase:current.phase,activated,rendererReady,visible});
    });
  }
  window.on('move',()=>{
    const b=window.getBounds(),a=screen.getDisplayMatching(b).workArea;
    if(current?.draggable && expected && (b.x!==expected.x || b.y!==expected.y)){emit({moved:true});expected=b;}
    const {x,y}=geometry.clamp(b,a);
    if(x!==b.x||y!==b.y)position({x,y});
  });
  void window.loadURL(page);
});
app.on('window-all-closed',()=>app.quit());

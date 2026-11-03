const {contextBridge,ipcRenderer}:typeof import('electron')=require('electron');
type Snapshot={capture:string;state:string};
contextBridge.exposeInMainWorld('apparentShare',Object.freeze({
 subscribe:(callback:(state:Snapshot)=>void)=>{const receive=(_event:import('electron').IpcRendererEvent,state:Snapshot)=>callback(state);ipcRenderer.on('share:state',receive);void ipcRenderer.invoke('share:ready').then(state=>{if(state)callback(state);});return()=>ipcRenderer.removeListener('share:state',receive);},
 confirm:()=>ipcRenderer.invoke('share:confirm'),cancel:()=>ipcRenderer.invoke('share:cancel'),
}));

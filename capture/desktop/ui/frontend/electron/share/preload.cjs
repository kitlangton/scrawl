const {contextBridge,ipcRenderer}=require('electron');
contextBridge.exposeInMainWorld('apparentShare',Object.freeze({
 subscribe:callback=>{const receive=(_event,state)=>callback(state);ipcRenderer.on('share:state',receive);void ipcRenderer.invoke('share:ready').then(state=>{if(state)callback(state);});return()=>ipcRenderer.removeListener('share:state',receive);},
 confirm:()=>ipcRenderer.invoke('share:confirm'),cancel:()=>ipcRenderer.invoke('share:cancel'),
}));

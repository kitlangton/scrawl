'use strict';
const {contextBridge,ipcRenderer} = require('electron');
contextBridge.exposeInMainWorld('apparentPill', Object.freeze({
  read: () => ipcRenderer.invoke('pill:read'),
  layout: revision => ipcRenderer.invoke('pill:layout',revision),
  manualDrag: process.platform==='linux',
  beginDrag: revision => ipcRenderer.invoke('pill:begin-drag',revision),
  beginPaste: revision => ipcRenderer.invoke('pill:begin-paste',revision),
  cancelPaste: () => ipcRenderer.invoke('pill:cancel-paste'),
  act: (revision, action, drop=false) => ipcRenderer.invoke('pill:act',revision,action,drop),
  subscribe: callback => {
    const listener = (_event, state) => callback(state);
    ipcRenderer.on('pill:state',listener);
    return () => ipcRenderer.removeListener('pill:state',listener);
  },
}));

// Two fixed signals only; the renderer cannot submit error text or properties.
contextBridge.exposeInMainWorld('apparentExceptions', Object.freeze({
  exception: () => ipcRenderer.send('apparent:unhandled', 'renderer_exception'),
  rejection: () => ipcRenderer.send('apparent:unhandled', 'renderer_rejection'),
}));

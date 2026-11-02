const { contextBridge, ipcRenderer } = require('electron');

// No generic invoke/send, paths, revisions or dialog options from the renderer.
contextBridge.exposeInMainWorld('apparent', Object.freeze({
  readRuntimeState: () => ipcRenderer.invoke('apparent:read-runtime-state'),
  readCaptureLocation: () => ipcRenderer.invoke('apparent:read-capture-location'),
  changeCaptureLocation: () => ipcRenderer.invoke('apparent:change-capture-location'),
  resetCaptureLocation: () => ipcRenderer.invoke('apparent:reset-capture-location'),
}));

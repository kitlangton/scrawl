const { contextBridge, ipcRenderer } = require('electron');

// No generic invoke/send, paths, arbitrary payloads or mutation methods.
contextBridge.exposeInMainWorld('apparent', Object.freeze({
  readRuntimeState: () => ipcRenderer.invoke('apparent:read-runtime-state'),
}));

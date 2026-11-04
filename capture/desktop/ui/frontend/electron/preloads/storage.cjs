const { contextBridge, ipcRenderer } = require('electron');

// Installed Storage has no Connect, reporting, clipboard or runtime capabilities.
contextBridge.exposeInMainWorld('apparent', Object.freeze({
  readCaptureLocation: () => ipcRenderer.invoke('apparent:read-capture-location'),
  changeCaptureLocation: () => ipcRenderer.invoke('apparent:change-capture-location'),
  resetCaptureLocation: () => ipcRenderer.invoke('apparent:reset-capture-location'),
  readRetention: () => ipcRenderer.invoke('apparent:read-retention'),
  saveRetention: (preset) => ipcRenderer.invoke('apparent:save-retention', preset),
  resetRetention: () => ipcRenderer.invoke('apparent:reset-retention'),
  readCaptureUsage: () => ipcRenderer.invoke('apparent:read-capture-usage'),
  navigateSettings: (pane) => ipcRenderer.invoke('apparent:navigate-settings', pane),
  showCaptureFolder: () => ipcRenderer.invoke('apparent:show-capture-folder'),
}));

contextBridge.exposeInMainWorld('apparentExceptions', Object.freeze({
  exception: () => ipcRenderer.send('apparent:unhandled', 'renderer_exception'),
  rejection: () => ipcRenderer.send('apparent:unhandled', 'renderer_rejection'),
}));

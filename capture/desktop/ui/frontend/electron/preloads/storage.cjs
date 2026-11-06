const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('apparentSettingsMounted', async () => {
  // The hidden page gets one render opportunity between these callbacks.
  // This recovers if Electron omits ready-to-show without showing a blank page.
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  await ipcRenderer.invoke('apparent:settings-mounted');
  await ipcRenderer.invoke('apparent:settings-frame-ready');
});
contextBridge.exposeInMainWorld('apparentBuild', Object.freeze({
  read: () => ipcRenderer.invoke('apparent:build-identity'),
}));

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

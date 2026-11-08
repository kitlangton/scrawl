const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('apparentSettingsMounted', async () => {
  // React has committed its DOM. Do not hold that fact behind hidden-page
  // animation scheduling: Chromium may already have supplied first paint.
  await ipcRenderer.invoke('apparent:settings-mounted');
  // The hidden page gets a render opportunity between these callbacks. Keep
  // this in the isolated preload, independent of the page's animation helpers.
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  await ipcRenderer.invoke('apparent:settings-frame-ready');
});

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

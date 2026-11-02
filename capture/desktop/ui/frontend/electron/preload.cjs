const { contextBridge, ipcRenderer } = require('electron');

// No generic invoke/send, paths, revisions or dialog options from the renderer.
contextBridge.exposeInMainWorld('apparent', Object.freeze({
  readRuntimeState: () => ipcRenderer.invoke('apparent:read-runtime-state'),
  readCaptureReference: () => ipcRenderer.invoke('apparent:read-capture-reference'),
  copyCaptureReference: () => ipcRenderer.invoke('apparent:copy-capture-reference'),
  readCaptureLocation: () => ipcRenderer.invoke('apparent:read-capture-location'),
  changeCaptureLocation: () => ipcRenderer.invoke('apparent:change-capture-location'),
  resetCaptureLocation: () => ipcRenderer.invoke('apparent:reset-capture-location'),
  readErrorReporting: () => ipcRenderer.invoke('apparent:read-error-reporting'),
  setErrorReporting: (enabled) => ipcRenderer.invoke('apparent:set-error-reporting', enabled),
  readConnectSettings: () => ipcRenderer.invoke('apparent:read-connect-settings'),
  connectSettings: () => ipcRenderer.invoke('apparent:connect-connect'),
  allowConnectUploads: () => ipcRenderer.invoke('apparent:connect-allow'),
  cancelConnectSettings: () => ipcRenderer.invoke('apparent:connect-cancel'),
  checkConnectConnection: () => ipcRenderer.invoke('apparent:connect-check'),
  disableConnectUploads: () => ipcRenderer.invoke('apparent:connect-disable'),
  disconnectConnectSettings: () => ipcRenderer.invoke('apparent:connect-disconnect'),
  openConnectBrowser: () => ipcRenderer.invoke('apparent:connect-open-browser'),
  readRetention: () => ipcRenderer.invoke('apparent:read-retention'),
  saveRetention: (preset) => ipcRenderer.invoke('apparent:save-retention', preset),
  resetRetention: () => ipcRenderer.invoke('apparent:reset-retention'),
  readCaptureUsage: () => ipcRenderer.invoke('apparent:read-capture-usage'),
}));

const { contextBridge, ipcRenderer } = require('electron');

// No generic invoke/send, paths, revisions or dialog options from the renderer.
contextBridge.exposeInMainWorld('apparent', Object.freeze({
  readDrawing: () => ipcRenderer.invoke('apparent:read-drawing'),
  undoDrawing: () => ipcRenderer.invoke('apparent:undo-drawing'),
  readMicrophoneEnabled: () => ipcRenderer.invoke('apparent:read-microphone-enabled'),
  setMicrophoneEnabled: (enabled) => ipcRenderer.invoke('apparent:set-microphone-enabled', enabled),
  readRuntimeState: () => ipcRenderer.invoke('apparent:read-runtime-state'),
  readCaptureReference: () => ipcRenderer.invoke('apparent:read-capture-reference'),
  copyCaptureReference: () => ipcRenderer.invoke('apparent:copy-capture-reference'),
  startPublicShare: () => ipcRenderer.invoke('apparent:start-public-share'),
  readPublicShare: () => ipcRenderer.invoke('apparent:read-public-share'),
  cancelPublicShare: () => ipcRenderer.invoke('apparent:cancel-public-share'),
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
  requestDisableConnectUploads: () => ipcRenderer.invoke('apparent:connect-request-disable'),
  requestDisconnectConnectSettings: () => ipcRenderer.invoke('apparent:connect-request-disconnect'),
  openConnectSharedLinks: () => ipcRenderer.invoke('apparent:connect-open-shared-links'),
  openConnectBrowser: () => ipcRenderer.invoke('apparent:connect-open-browser'),
  readRetention: () => ipcRenderer.invoke('apparent:read-retention'),
  saveRetention: (preset) => ipcRenderer.invoke('apparent:save-retention', preset),
  resetRetention: () => ipcRenderer.invoke('apparent:reset-retention'),
  readCaptureUsage: () => ipcRenderer.invoke('apparent:read-capture-usage'),
  navigateSettings: (pane) => ipcRenderer.invoke('apparent:navigate-settings', pane),
  showCaptureFolder: () => ipcRenderer.invoke('apparent:show-capture-folder'),
}));

// Two fixed signals only; the renderer cannot submit error text or properties.
contextBridge.exposeInMainWorld('apparentExceptions', Object.freeze({
  exception: () => ipcRenderer.send('apparent:unhandled', 'renderer_exception'),
  rejection: () => ipcRenderer.send('apparent:unhandled', 'renderer_rejection'),
}));

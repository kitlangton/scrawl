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

// Installed Connect exposes no Storage, clipboard, reporting or capture commands.
contextBridge.exposeInMainWorld('apparent', Object.freeze({
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
  navigateSettings: (pane) => ipcRenderer.invoke('apparent:navigate-settings', pane),
}));

contextBridge.exposeInMainWorld('apparentExceptions', Object.freeze({
  exception: () => ipcRenderer.send('apparent:unhandled', 'renderer_exception'),
  rejection: () => ipcRenderer.send('apparent:unhandled', 'renderer_rejection'),
}));

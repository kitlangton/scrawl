const {contextBridge, ipcRenderer} = require('electron');
contextBridge.exposeInMainWorld('apparentBuild', Object.freeze({
  read: () => ipcRenderer.invoke('apparent:build-identity'),
}));
// Parent state may arrive before React's first effect. Keep one current value
// per presentation message so a late subscription receives the saved shortcut.
const latest = new Map();
const listeners = new Set();
ipcRenderer.on('onboarding:view', (_event, value) => {
  if (!['state', 'shortcut', 'shortcut-editor'].includes(value?.type)) return;
  latest.set(value.type, value);
  for (const listener of listeners) listener(value);
});
contextBridge.exposeInMainWorld('onboarding', {
  action: action => ipcRenderer.invoke('onboarding:action', action),
  subscribe: listener => {
    listeners.add(listener);
    for (const value of latest.values()) listener(value);
    return () => listeners.delete(listener);
  },
});

// Two fixed signals only; the renderer cannot submit error text or properties.
contextBridge.exposeInMainWorld('apparentExceptions', Object.freeze({
  exception: () => ipcRenderer.send('apparent:unhandled', 'renderer_exception'),
  rejection: () => ipcRenderer.send('apparent:unhandled', 'renderer_rejection'),
}));

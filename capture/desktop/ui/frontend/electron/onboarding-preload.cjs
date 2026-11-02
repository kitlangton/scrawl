const {contextBridge, ipcRenderer} = require('electron');
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

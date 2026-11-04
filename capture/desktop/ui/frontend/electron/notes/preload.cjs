const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('captureNotes', Object.freeze({
  onSnapshot: (callback) => {
    const receive = (_event, state) => callback(state);
    ipcRenderer.on('notes:snapshot', receive);
    ipcRenderer.invoke('notes:ready');
    return () => ipcRenderer.removeListener('notes:snapshot', receive);
  },
  command: (message) => ipcRenderer.invoke('notes:command', message),
}));

// Ordered behind this renderer's already-issued save invocations. Not exposed to page JS.
ipcRenderer.on('notes:finish', () => { void ipcRenderer.invoke('notes:finish-ready'); });

// Two fixed signals only; the renderer cannot submit error text or properties.
contextBridge.exposeInMainWorld('apparentExceptions', Object.freeze({
  exception: () => ipcRenderer.send('apparent:unhandled', 'renderer_exception'),
  rejection: () => ipcRenderer.send('apparent:unhandled', 'renderer_rejection'),
}));

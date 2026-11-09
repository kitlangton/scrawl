import { contextBridge, ipcRenderer } from 'electron';
contextBridge.exposeInMainWorld('hermesRecording', {
  read: () => ipcRenderer.invoke('apparent:hermes-read'),
  start: () => ipcRenderer.invoke('apparent:hermes-start'),
});

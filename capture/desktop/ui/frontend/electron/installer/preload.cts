import {contextBridge, ipcRenderer} from 'electron';
contextBridge.exposeInMainWorld('apparentInstaller', Object.freeze({
  action: (action: string, replace?: boolean) => ipcRenderer.invoke('installer:action',
    replace === undefined ? {action} : {action, replace}),
}));

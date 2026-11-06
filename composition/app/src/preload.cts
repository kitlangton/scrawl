import { contextBridge, ipcRenderer } from 'electron';
import type { Revision, SaveResult } from '@apparent/app-contracts/host';

const api = Object.freeze({
  onboardingProgress: Object.freeze({
    read: () => ipcRenderer.invoke('apparent:onboarding-progress:read') as Promise<{ revision: Revision; value: unknown }>,
    compareAndSwap: (expected: Revision, value: unknown) => ipcRenderer.invoke(
      'apparent:onboarding-progress:compare-and-swap', { expected, value },
    ) as Promise<SaveResult<unknown>>,
  }),
});
contextBridge.exposeInMainWorld('apparent', api);

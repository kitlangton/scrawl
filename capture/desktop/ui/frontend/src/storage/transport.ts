import type {LocationPorts, RetentionPorts} from '@apparent/record-storage';

// Desktop adapts its scoped preload. Record owns policy and readback sequencing.
export const locationPorts: LocationPorts = {
  read: async () => await window.apparent?.readCaptureLocation() ?? {status: 'disabled'},
  change: async () => await window.apparent?.changeCaptureLocation() ?? {status: 'disabled'},
  reset: async () => await window.apparent?.resetCaptureLocation() ?? {status: 'disabled'},
  show: async () => { await window.apparent?.showCaptureFolder(); },
};
export const retentionPorts: RetentionPorts = {
  read: async () => await window.apparent?.readRetention() ?? {status: 'disabled'},
  save: async policy => await window.apparent?.saveRetention(policy) ?? {status: 'disabled'},
  reset: async () => await window.apparent?.resetRetention() ?? {status: 'disabled'},
  usage: async () => await window.apparent?.readCaptureUsage() ?? {status: 'disabled'},
};

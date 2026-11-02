export type RuntimeState =
  | { status: 'connected'; phase: 'idle' | 'selecting' | 'starting' | 'recording' | 'processing' | 'completed' }
  | { status: 'disconnected' }
  | { status: 'error'; message: string };

declare global {
  interface Window {
    apparent?: {
      readRuntimeState: () => Promise<RuntimeState>;
      readCaptureLocation: () => Promise<CaptureLocation>;
      changeCaptureLocation: () => Promise<CaptureLocation>;
      resetCaptureLocation: () => Promise<CaptureLocation>;
      readConnectSettings: () => Promise<ConnectSettings>;
      connectSettings: () => Promise<ConnectSettings>;
      allowConnectUploads: () => Promise<ConnectSettings>;
      cancelConnectSettings: () => Promise<ConnectSettings>;
      checkConnectConnection: () => Promise<ConnectSettings>;
      disableConnectUploads: () => Promise<ConnectSettings>;
      disconnectConnectSettings: () => Promise<ConnectSettings>;
      openConnectBrowser: () => Promise<ConnectSettings>;
    };
  }
}

export type LocationSnapshot = {
  savedPath: string | null; effectivePath: string; displayPath: string; custom: boolean;
  source: 'environment' | 'configured' | 'builtin' | 'fallback';
};
export type CaptureLocation =
  | { status: 'connected'; storage: LocationSnapshot }
  | { status: 'disabled' | 'disconnected' | 'incompatible' | 'cancelled' }
  | { status: 'error'; code: string };

export type ConnectAction = 'connect' | 'allow' | 'cancel' | 'check' | 'disable' | 'disconnect' | 'open-browser';
export type ConnectView = {
  title: string; status: string; detail: string; server: string | null;
  verificationUri: string | null; userCode: string | null;
  buttons: { command: ConnectAction; title: string; enabled: boolean }[]; note: string;
};
export type ConnectSettings =
  | { status: 'connected'; view: ConnectView }
  | { status: 'disabled' | 'disconnected' | 'cancelled' }
  | { status: 'error' };

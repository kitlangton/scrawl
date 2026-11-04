export type RuntimeState =
  | { status: 'connected'; phase: 'idle' | 'selecting' | 'starting' | 'recording' | 'processing' | 'completed' }
  | { status: 'disconnected' }
  | { status: 'error'; message: string };

declare global {
  interface Window {
    apparent?: {
      readDrawing: () => Promise<Drawing>;
      undoDrawing: () => Promise<Drawing>;
      readMicrophoneEnabled: () => Promise<ErrorReporting>;
      setMicrophoneEnabled: (enabled: boolean) => Promise<ErrorReporting>;
      readRuntimeState: () => Promise<RuntimeState>;
      readCaptureReference: () => Promise<CaptureReference>;
      copyCaptureReference: () => Promise<CaptureReference>;
      startPublicShare: () => Promise<PublicShareResult>;
      readPublicShare: () => Promise<PublicShareResult>;
      cancelPublicShare: () => Promise<PublicShareResult>;
      readCaptureLocation: () => Promise<CaptureLocation>;
      changeCaptureLocation: () => Promise<CaptureLocation>;
      resetCaptureLocation: () => Promise<CaptureLocation>;
      readErrorReporting: () => Promise<ErrorReporting>;
      setErrorReporting: (enabled: boolean) => Promise<ErrorReporting>;
      readConnectSettings: () => Promise<ConnectSettings>;
      connectSettings: () => Promise<ConnectSettings>;
      allowConnectUploads: () => Promise<ConnectSettings>;
      cancelConnectSettings: () => Promise<ConnectSettings>;
      checkConnectConnection: () => Promise<ConnectSettings>;
      disableConnectUploads: () => Promise<ConnectSettings>;
      disconnectConnectSettings: () => Promise<ConnectSettings>;
      requestDisableConnectUploads: () => Promise<ConnectSettings>;
      requestDisconnectConnectSettings: () => Promise<ConnectSettings>;
      openConnectSharedLinks: () => Promise<ConnectSettings>;
      openConnectBrowser: () => Promise<ConnectSettings>;
      readRetention: () => Promise<RetentionResult>;
      saveRetention: (preset: string) => Promise<RetentionResult>;
      resetRetention: () => Promise<RetentionResult>;
      readCaptureUsage: () => Promise<UsageResult>;
      navigateSettings: (pane: string) => Promise<void>;
      showCaptureFolder: () => Promise<void>;
    };
  }
}

export type LocationSnapshot = {
  savedPath: string | null; effectivePath: string; displayPath: string; custom: boolean;
  source: 'environment' | 'configured' | 'builtin' | 'fallback';
};

export type CaptureReference =
  | { status: 'connected'; id: string | null; copied: boolean }
  | { status: 'disabled' | 'disconnected' | 'incompatible' | 'cancelled' }
  | { status: 'error'; code: string };
export type CaptureLocation =
  | { status: 'connected'; storage: LocationSnapshot }
  | { status: 'disabled' | 'disconnected' | 'incompatible' | 'cancelled' }
  | { status: 'error'; code: string };

export type ErrorReporting = { status: 'connected'; enabled: boolean } |
  { status: 'disconnected' | 'incompatible' } | { status: 'error'; code: string };
export type ConnectAction = import('../../../../../connect/client/typescript/src/settings-presentation').Action;
export type ConnectView = {
  title: string; status: string; detail: string; server: string | null;
  verificationUri: string | null; userCode: string | null;
  buttons: ConnectButton[]; note: string;
  presentationState?: import('../../../../../connect/client/typescript/src/settings-presentation').PresentationState;
  presentation?: import('../../../../../connect/client/typescript/src/settings-presentation').Presentation;
};
export type ConnectButton = import('../../../../../connect/client/typescript/src/settings-presentation').Button;
export type ConnectSettings =
  | { status: 'connected'; view: ConnectView }
  | { status: 'disabled' | 'disconnected' | 'cancelled' }
  | { status: 'error' };
type Unavailable = Exclude<CaptureLocation, { status: 'connected' }>;
export type RetentionResult = Unavailable | { status: 'connected'; custom: boolean;
  retention: { preset: string | null; options: { id: string; label: string }[]; note: string } };
export type UsageResult = Unavailable | { status: 'connected';
  usage: { effectivePath: string; displayPath: string; formatted: string } };

export type Drawing = { status: "connected"; active: boolean; strokes: number } | { status: "disconnected" | "cancelled" } | { status: "error"; code: string };

export type PublicShareResult = { status:'connected'; state:'preparing'|'prepared'|'copied'|'failed'|'cancelled'|'expired' } | {status:'error';code:string} | {status:'cancelled'};

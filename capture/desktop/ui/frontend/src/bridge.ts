export type RuntimeState =
  | { status: 'connected'; phase: 'idle' | 'selecting' | 'starting' | 'recording' | 'processing' | 'completed' }
  | { status: 'disconnected' }
  | { status: 'error'; message: string };

declare global {
  interface Window {
    apparent?: { readRuntimeState: () => Promise<RuntimeState> };
  }
}

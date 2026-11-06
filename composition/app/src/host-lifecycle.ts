import type { BrowserWindow } from 'electron';

/** The host, rather than renderer state, decides whether native ownership is clear. */
export function bindWindowShutdown(options: {
  window: BrowserWindow;
  canClose(): Promise<boolean>;
  close(): Promise<void>;
  revoke(): void;
  refused(reason: 'ownership-unresolved' | 'shutdown-failed'): void;
}): { settled(): Promise<void> } {
  let phase: 'open' | 'checking' | 'closed' = 'open';
  let shutdown: Promise<void> = Promise.resolve();
  let revoked = false;
  const revoke = () => { if (!revoked) { revoked = true; options.revoke(); } };
  const attempt = async () => {
    try {
      if (!await options.canClose()) { options.refused('ownership-unresolved'); phase = 'open'; return; }
      revoke();
      await options.close();
      phase = 'closed';
      if (!options.window.isDestroyed()) options.window.close();
    } catch {
      phase = 'open';
      options.refused('shutdown-failed');
    }
  };
  options.window.on('close', event => {
    if (phase === 'closed') return;
    event.preventDefault();
    if (phase === 'checking') return;
    phase = 'checking';
    shutdown = attempt();
  });
  options.window.once('closed', () => {
    revoke();
    if (phase !== 'closed') {
      shutdown = shutdown.then(async () => {
        if (phase === 'closed') return;
        if (!await options.canClose()) throw new Error('Native ownership remains unresolved');
        await options.close();
        phase = 'closed';
      });
      // The main host consumes settled(); never turn this into quit clearance.
      void shutdown.catch(() => {});
    }
  });
  return { settled: () => shutdown };
}

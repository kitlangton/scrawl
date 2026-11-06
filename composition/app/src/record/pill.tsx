import { useEffect, useState } from 'react';
import type { ComponentType } from 'react';
import type { RecordClient } from '@apparent/record/capabilities';
import { createRecordClientPillHost, disposeWorkflowPillHost, RecordingPill, type PillHost } from '@apparent/record/ui';
import type { RecordRendererPort } from './wire.js';
import { connectRecordClient } from './renderer-client.js';

export interface RecordPillScreenProps {
  /** Injected narrow renderer capability. It is never discovered from window or renderer globals. */
  port?: RecordRendererPort;
}

export function RecordPillScreen({ port }: RecordPillScreenProps) {
  const [connection, setConnection] = useState<{ port: RecordRendererPort; host: PillHost }>();

  useEffect(() => {
    let active = true;
    let client: RecordClient | undefined;
    let host: PillHost | undefined;
    if (!port) {
      setConnection(undefined);
      return () => { active = false; };
    }

    void connectRecordClient(port).then(connected => {
      client = connected;
      if (!active) { connected.detach(); return; }
      host = createRecordClientPillHost(connected, { createRequestId: () => globalThis.crypto.randomUUID() });
      if (!active) {
        disposeWorkflowPillHost(host);
        connected.detach();
        return;
      }
      setConnection({ port, host });
    }).catch(() => {
      try { if (host) disposeWorkflowPillHost(host); }
      finally { client?.detach(); }
      if (active) setConnection(undefined);
    });

    return () => {
      active = false;
      try { if (host) disposeWorkflowPillHost(host); }
      // Detaches this renderer observer only; RecordClient.detach never detaches the main provider.
      finally { client?.detach(); }
    };
  }, [port]);

  return <RecordingPill host={port && connection?.port === port ? connection.host : undefined} />;
}

function RecordSettings() { return null; }

const Screen: ComponentType<RecordPillScreenProps> = RecordPillScreen;
export default { id: 'record', Screen, Settings: RecordSettings };

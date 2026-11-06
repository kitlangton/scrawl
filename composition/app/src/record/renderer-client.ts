import type { CapabilitySnapshot, CommandResult, RecordClient, RecordCommand } from '@apparent/record/capabilities';
import type { RecordRendererPort } from './wire.js';
import { cloneCapabilitySnapshot, cloneCommandResult, cloneRecordCommand } from './wire.js';

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

/** Adds the RecordClient immediate-observation contract to the renderer IPC port. */
export async function connectRecordClient(port: RecordRendererPort): Promise<RecordClient> {
  if (!port || typeof port.read !== 'function' || typeof port.execute !== 'function' || typeof port.observe !== 'function') {
    throw new TypeError('Invalid Record renderer port');
  }
  let current: CapabilitySnapshot | undefined;
  const retiredEpochs = new Set<string>();
  const observers = new Set<(snapshot: CapabilitySnapshot) => void>();
  let detached = false;

  const publish = (value: unknown): CapabilitySnapshot | undefined => {
    if (detached) return current;
    const next = cloneCapabilitySnapshot(value);
    if (!next) return undefined;
    if (current) {
      if (next.providerEpoch === current.providerEpoch) {
        if (next.revision <= current.revision) return current;
      } else {
        if (retiredEpochs.has(next.providerEpoch) || retiredEpochs.size >= 512) return current;
        retiredEpochs.add(current.providerEpoch);
      }
    }
    current = next;
    for (const listener of [...observers]) {
      try { listener(clone(current)); }
      catch { observers.delete(listener); }
    }
    return current;
  };

  let unsubscribePort: (() => void) | undefined;
  try {
    unsubscribePort = port.observe(snapshot => { publish(snapshot); });
    if (typeof unsubscribePort !== 'function') throw new TypeError('Record port returned no observer cleanup');
    const initial = await port.read();
    if (!publish(initial)) throw new TypeError('Record port returned an invalid initial snapshot');
    if (!current) throw new TypeError('Record port did not provide a complete snapshot');
  } catch (error) {
    try { unsubscribePort?.(); } catch { /* Preserve the handshake error. */ }
    throw error;
  }

  const client: RecordClient = {
    observe(listener) {
      if (typeof listener !== 'function') throw new TypeError('Record observer must be a function');
      if (detached || !current) throw new Error('Record renderer client is detached');
      observers.add(listener);
      try { listener(clone(current)); }
      catch { observers.delete(listener); }
      return () => { observers.delete(listener); };
    },
    async execute(command: RecordCommand): Promise<CommandResult> {
      const safeCommand = cloneRecordCommand(command);
      if (!safeCommand) throw new TypeError('Invalid Record command DTO');
      if (detached || !current) {
        if (!current) throw new Error('Record renderer client has no provider snapshot');
        return { status: 'refused', requestId: safeCommand.requestId, reason: 'Record renderer client is detached', snapshot: clone(current) };
      }
      try {
        const raw = await port.execute(safeCommand);
        const result = cloneCommandResult(raw, safeCommand.requestId);
        if (!result || !current) return { status: 'outcome-unknown', requestId: safeCommand.requestId, snapshot: current ? clone(current) : clone(initial) };
        const latest = publish(result.snapshot) ?? current;
        return { ...result, snapshot: clone(latest) };
      } catch {
        // invoke rejection cannot establish whether main or the provider acted.
        return { status: 'outcome-unknown', requestId: safeCommand.requestId, snapshot: clone(current) };
      }
    },
    detach() {
      if (detached) return;
      detached = true;
      observers.clear();
      try { unsubscribePort?.(); } catch { /* Detach only this observer; never detach Record or stop capture. */ }
    },
  };
  const initial = clone(current!);
  return client;
}

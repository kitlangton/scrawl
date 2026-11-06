import { randomUUID } from 'node:crypto';
import {
  createRecordCapabilityProvider,
  type RecordCapabilityProviderOptions,
  type RecordClient,
} from '@apparent/record/capabilities';
import type { RecordHostServices } from './host.ts';
import type { RecordNativeBindings } from './native-bindings.ts';

type Runtime = RecordCapabilityProviderOptions['runtime'];
export interface RecordProviderHostOptions {
  /** Exclusive command/observation ownership transfers to this host on success. */
  runtime: Runtime;
  capture2Start?: RecordCapabilityProviderOptions['capture2Start'];
  /** Authenticated Record output facts; omission remains unavailable. */
  captureFacts?: RecordCapabilityProviderOptions['captureFacts'];
  nativeBindings: RecordNativeBindings;
  /** Releases only this host's Engine resources after clearance. Must be idempotent; failure is not clearance. */
  closeEngine(): Promise<void>;
}

type Phase = 'open' | 'admitted-close' | 'provider-detached' | 'engine-closed' | 'closed' | 'cleanup-unknown';
const unresolved = new Set(['starting', 'recording', 'processing', 'unknown']);

/** Trusted main-process assembly of the real Record provider, with one admission/close boundary. */
export function createRecordProviderHost(options: RecordProviderHostOptions): RecordHostServices {
  if (!options?.runtime || typeof options.runtime.unresolvedOwnership !== 'function'
    || !options.nativeBindings || typeof options.nativeBindings.close !== 'function'
    || typeof options.closeEngine !== 'function') throw new TypeError('Invalid Record provider host options');
  const runtime = options.runtime;
  const bindings = options.nativeBindings;
  const closeEngine = options.closeEngine;
  const captureFacts = options.captureFacts;
  const provider = createRecordCapabilityProvider({ runtime, capture2Start: options.capture2Start,
    captureFacts: options.captureFacts, native: bindings.native, penConfig: bindings.penConfig,
    clipboard: bindings.clipboard, providerEpoch: randomUUID() });
  let phase: Phase = 'open';
  let commands = 0;
  let closing: Promise<void> | undefined;
  let subscriptions: Set<() => void> | undefined = new Set();
  const currentPhase = (): Phase => phase;

  const record: RecordClient = {
    observe(listener) {
      if (!subscriptions) throw new Error('Record client is detached');
      const removeProvider = provider.observe(listener);
      if (!subscriptions) { removeProvider(); return () => {}; }
      const remove = () => { removeProvider(); subscriptions?.delete(remove); };
      subscriptions.add(remove);
      return remove;
    },
    async execute(command) {
      if (!subscriptions || phase !== 'open') return { status: 'refused', requestId: command.requestId,
        reason: 'Record host is closing', snapshot: provider.current() };
      commands++;
      try { return await provider.execute(command); }
      finally { commands--; }
    },
    // UI detach is observation-only. The bridge owns its individual unsubscribe.
    // Provider/runtime disposal belongs exclusively to the cleared host shutdown.
    detach() {
      const previous = subscriptions;
      subscriptions = undefined;
      for (const remove of previous ?? []) remove();
    },
  };

  const canClose = async (): Promise<boolean> => {
    if (phase === 'cleanup-unknown') return false;
    if (phase !== 'open') return true;
    if (commands) return false;
    try {
      // The provider refreshes asynchronously, including at construction. Its
      // current renderer snapshot cannot clear ownership not yet read from the
      // authoritative output/reservation source. Missing producer is distinct
      // from a configured producer whose current facts are unavailable.
      if (captureFacts) {
        const facts = await captureFacts.readFacts();
        if (!facts || facts.version !== 1 || !Array.isArray(facts.captures)
          || !['idle', 'consumed', 'cancelled', 'expired'].includes(facts.nextShortcutAdmission?.state)
          || facts.captures.some(capture => !['saved', 'cancelled', 'failed'].includes(capture.phase))) return false;
        const admission = facts.nextShortcutAdmission;
        if (admission.state === 'consumed'
          && !facts.captures.some(capture => capture.captureId === admission.captureId)) return false;
      }
      // A renderer command may have entered while the authoritative read awaited.
      if (commands || currentPhase() === 'cleanup-unknown') return false;
      if (currentPhase() !== 'open') return true;
      const report = runtime.unresolvedOwnership();
      if (!report || report.storageUnavailable !== false || report.reconciliation !== 'complete'
        || !Array.isArray(report.requestIds) || report.requestIds.length) return false;
      const current = provider.current();
      if (current.captures.some(capture => unresolved.has(capture.phase))
        || current.nextShortcutAdmission.state === 'armed') return false;
    } catch { return false; }
    // No await between the final report and closing admission. Commands already
    // queued through this host count above; later calls cannot reach the provider.
    phase = 'admitted-close';
    return true;
  };

  return {
    record,
    canClose,
    async close() {
      if (phase === 'closed') return;
      if (phase === 'cleanup-unknown') throw new Error('Record cleanup ownership remains unknown');
      if (phase === 'open' && !await canClose()) throw new Error('Record ownership remains unresolved');
      if (closing) return closing;
      const cleanup = async () => {
        if (phase === 'admitted-close') {
          try { record.detach(); provider.detach(); phase = 'provider-detached'; }
          catch (error) { phase = 'cleanup-unknown'; throw error; }
        }
        if (phase === 'provider-detached') { await closeEngine(); phase = 'engine-closed'; }
        if (phase === 'engine-closed') { await bindings.close(); phase = 'closed'; }
      };
      closing = cleanup();
      try { await closing; }
      finally { closing = undefined; }
    },
  };
}

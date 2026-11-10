import type {NativeRetentionResult, RetentionPolicy, RetentionPreset, RetentionResult, UsageResult} from './types.js';
import {isStoredRetention, retentionForPreset, retentionView} from './policy.js';

export type RetentionPorts = {
  read(): Promise<NativeRetentionResult>;
  save(policy: RetentionPolicy): Promise<NativeRetentionResult>;
  reset(): Promise<NativeRetentionResult>;
  usage(): Promise<UsageResult>;
};
export type RetentionState = {result?: RetentionResult; usage?: UsageResult; preset: RetentionPreset | ''; busy: boolean; scanning: boolean};

export class RetentionController {
  #ports: RetentionPorts;
  #path: string;
  #state: RetentionState = {preset: '', busy: true, scanning: false};
  #generation = 0;
  #usageGeneration = 0;
  #closed = false;
  #mutating = false;
  #listeners = new Set<(state: RetentionState) => void>();
  constructor(ports: RetentionPorts, effectivePath: string) { this.#ports = ports; this.#path = effectivePath; }
  get state(): RetentionState { return this.#state; }
  subscribe(listener: (state: RetentionState) => void): () => void {
    if (this.#closed) return () => {};
    this.#listeners.add(listener); listener(this.#state);
    return () => this.#listeners.delete(listener);
  }
  #publish(patch: Partial<RetentionState>) {
    this.#state = {...this.#state, ...patch};
    for (const listener of this.#listeners) listener(this.#state);
  }
  select(preset: string) {
    if (!this.#closed && !this.#state.busy && !this.#state.scanning &&
        ['week', 'month', 'forever'].includes(preset)) this.#publish({preset: preset as RetentionPreset});
  }
  async measure(): Promise<void> {
    if (this.#closed || this.#state.busy) return;
    const generation = ++this.#usageGeneration;
    const current = () => !this.#closed && generation === this.#usageGeneration;
    this.#publish({scanning: true, usage: undefined});
    try {
      let usage = await this.#ports.usage();
      if (!current()) return;
      if (usage.status === 'connected' && usage.usage.effectivePath !== this.#path) usage = {status: 'error', code: 'conflict'};
      this.#publish({usage});
    } catch { if (current()) this.#publish({usage: {status: 'error', code: 'bridge_failed'}}); }
    finally { if (current()) this.#publish({scanning: false}); }
  }
  async request(operation: 'read' | 'save' | 'reset'): Promise<void> {
    if (!['read', 'save', 'reset'].includes(operation)) return;
    if (this.#closed || this.#mutating) return;
    if (operation !== 'read' && (this.#state.busy || this.#state.result?.status !== 'connected')) return;
    const preset = this.#state.preset;
    if (operation === 'save' && !preset) return;
    const generation = ++this.#generation;
    this.#mutating = operation !== 'read';
    const current = () => !this.#closed && generation === this.#generation;
    this.#usageGeneration++;
    this.#publish({busy: true, scanning: false, usage: undefined, result: undefined});
    try {
      const raw = operation === 'read' ? await this.#ports.read() : operation === 'reset' ? await this.#ports.reset() : await this.#ports.save(retentionForPreset(preset as RetentionPreset));
      if (!current()) return;
      const result: RetentionResult = raw.status !== 'connected' ? raw : isStoredRetention(raw.policy)
        ? {status: 'connected', ...retentionView(raw.policy, raw.custom)} : {status: 'error', code: 'invalid_response'};
      this.#publish({result, preset: result.status === 'connected' ? result.retention.preset ?? '' : ''});
    } catch { if (current()) this.#publish({preset: '', result: {status: 'error', code: operation === 'read' ? 'bridge_failed' : 'outcome_unknown'}}); }
    finally { if (current()) { this.#mutating = false; this.#publish({busy: false}); void this.measure(); } }
  }
  dispose() { this.#closed = true; this.#generation++; this.#usageGeneration++; this.#listeners.clear(); }
}

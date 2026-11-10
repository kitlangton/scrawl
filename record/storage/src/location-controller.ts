import type {CaptureLocation, LocationSnapshot} from './types.js';
import {isLocationSnapshot} from './policy.js';

export type LocationPorts = {
  read(): Promise<CaptureLocation>;
  change(): Promise<CaptureLocation>;
  reset(): Promise<CaptureLocation>;
  show?(): Promise<void>;
};
export type LocationState = {result: CaptureLocation; snapshot?: LocationSnapshot; busy: boolean};

/** Request/readback policy only. Ports retain native chooser admission and execution. */
export class LocationController {
  #ports: LocationPorts;
  #state: LocationState = {result: {status: 'disabled'}, busy: true};
  #generation = 0;
  #closed = false;
  #mutating = false;
  #listeners = new Set<(state: LocationState) => void>();
  constructor(ports: LocationPorts) { this.#ports = ports; }
  get state(): LocationState { return this.#state; }
  subscribe(listener: (state: LocationState) => void): () => void {
    if (this.#closed) return () => {};
    this.#listeners.add(listener); listener(this.#state);
    return () => this.#listeners.delete(listener);
  }
  #publish(patch: Partial<LocationState>) {
    this.#state = {...this.#state, ...patch};
    for (const listener of this.#listeners) listener(this.#state);
  }
  async request(operation: 'read' | 'change' | 'reset'): Promise<void> {
    if (!['read', 'change', 'reset'].includes(operation)) return;
    if (this.#closed || this.#mutating) return;
    // Failed/unknown writes require authoritative readback before another intent.
    if (operation !== 'read' && (this.#state.busy || this.#state.result.status !== 'connected')) return;
    const generation = ++this.#generation;
    this.#mutating = operation !== 'read';
    const current = () => !this.#closed && generation === this.#generation;
    this.#publish({busy: true});
    try {
      let result = await this.#ports[operation]();
      if (!current()) return;
      if (result.status === 'connected' && !isLocationSnapshot(result.storage)) result = {status: 'error', code: 'invalid_response'};
      if (result.status !== 'cancelled') this.#publish({result, snapshot: result.status === 'connected' ? result.storage : undefined});
    } catch {
      if (current()) this.#publish({snapshot: undefined, result: {status: 'error', code: operation === 'read' ? 'bridge_failed' : 'outcome_unknown'}});
    } finally { if (current()) { this.#mutating = false; this.#publish({busy: false}); } }
  }
  dispose() {
    this.#closed = true; this.#generation++; this.#listeners.clear();
  }
  async show(): Promise<void> {
    if (this.#closed || this.#state.busy || this.#state.result.status !== 'connected') return;
    const generation = this.#generation;
    try { await this.#ports.show?.(); }
    catch { if (!this.#closed && generation === this.#generation) this.#publish({result: {status: 'error', code: 'bridge_failed'}}); }
  }
}

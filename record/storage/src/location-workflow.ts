import type {CaptureLocation, Unavailable, NativeLocationFacts} from './types.js';
import {locationCandidate, normalizeSelectedPath, resolveLocationCandidate} from './location-policy.js';

type FactsResult = Unavailable | {status: 'connected'; location: NativeLocationFacts};
type Resolved = Unavailable | {status: 'connected'; revision: string; storage: Extract<CaptureLocation, {status: 'connected'}>['storage']};
export type LocationWorkflowPorts = {
  facts(): Promise<FactsResult>;
  probe(path: string, expected: string): Promise<Unavailable | {status: 'connected'; available: boolean}>;
  choose(effectivePath: string): Promise<{canceled: boolean; filePaths: string[]}>;
  write(operation: 'set' | 'reset', fields: {expected: string; path?: string}): Promise<FactsResult>;
};

/** Record intent; host owns operation admission/lifetime and native owns execution. */
export class LocationWorkflow {
  private readonly ports: LocationWorkflowPorts;
  constructor(ports: LocationWorkflowPorts) { this.ports = ports; }
  async #resolve(facts: NativeLocationFacts): Promise<Resolved> {
    const candidate = locationCandidate(facts);
    if (candidate.snapshot) return {status: 'connected', revision: candidate.revision, storage: candidate.snapshot};
    const probe = await this.ports.probe(candidate.probePath!, candidate.revision);
    if (probe.status !== 'connected') return probe;
    return {status: 'connected', revision: candidate.revision, storage: resolveLocationCandidate(facts, probe.available)};
  }
  async #read(): Promise<Resolved> {
    const result = await this.ports.facts();
    if (result.status !== 'connected') return result;
    try { return await this.#resolve(result.location); }
    catch { return {status: 'error', code: 'invalid_response'}; }
  }
  #project(result: Resolved): CaptureLocation {
    return result.status === 'connected' ? {status: 'connected', storage: result.storage} : result;
  }
  async read(): Promise<CaptureLocation> { return this.#project(await this.#read()); }
  async mutate(operation: 'change' | 'reset', current: () => boolean): Promise<CaptureLocation> {
    let publicationAttempted = false;
    try {
      const before = await this.#read();
      if (!current()) return {status: 'cancelled'};
      if (before.status !== 'connected') return before;
      const fields: {expected: string; path?: string} = {expected: before.revision};
      if (operation === 'change') {
        const selected = await this.ports.choose(before.storage.effectivePath);
        if (!current() || selected.canceled) return {status: 'cancelled'};
        if (selected.filePaths.length !== 1) return {status: 'error', code: 'invalid_path'};
        fields.path = normalizeSelectedPath(selected.filePaths[0]);
        if (!fields.path) return {status: 'error', code: 'invalid_path'};
      }
      if (!current()) return {status: 'cancelled'};
      publicationAttempted = true;
      const saved = await this.ports.write(operation === 'change' ? 'set' : 'reset', fields);
      if (!current()) return {status: 'cancelled'};
      if (saved.status !== 'connected') return saved;
      const result = await this.#resolve(saved.location);
      return current() ? this.#project(result) : {status: 'cancelled'};
    } catch {
      if (!current()) return {status: 'cancelled'};
      return {status: 'error', code: publicationAttempted ? 'outcome_unknown' : 'bridge_failed'};
    }
  }
}

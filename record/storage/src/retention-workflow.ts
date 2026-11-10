import type {NativeRetentionResult, RetentionPolicy, UsageResult, Unavailable} from './types.js';
import {isStoredRetention, isRetentionPolicy} from './policy.js';

export type RevisionedRetention = Unavailable | (Extract<NativeRetentionResult, {status: 'connected'}> & {revision: string});
export type RetentionWorkflowPorts = {
  read(): Promise<RevisionedRetention>;
  write(operation: 'save' | 'reset', fields: {expected: string; policy?: RetentionPolicy}): Promise<RevisionedRetention>;
  usage(): Promise<UsageResult>;
};
/** Private revision intent; host supplies shared resource admission and lifetime. */
export class RetentionWorkflow {
  #ports: RetentionWorkflowPorts;
  #revision?: string;
  #epoch = 0;
  #pending?: {version: number; promise: Promise<RevisionedRetention>};
  constructor(ports: RetentionWorkflowPorts) { this.#ports = ports; }
  get ready() { return this.#revision !== undefined; }
  invalidate() { this.#epoch++; this.#revision = undefined; this.#pending = undefined; }
  #project(result: RevisionedRetention): NativeRetentionResult {
    this.#revision = undefined;
    if (result.status !== 'connected') return result;
    if (typeof result.revision !== 'string' || !/^[a-f0-9]{16}$/.test(result.revision) ||
        typeof result.custom !== 'boolean' || !isStoredRetention(result.policy))
      return {status: 'error', code: 'invalid_response'};
    this.#revision = result.revision;
    return {status: 'connected', custom: result.custom, policy: result.policy};
  }
  async read(version: number, current: () => boolean): Promise<NativeRetentionResult> {
    const epoch = this.#epoch;
    const valid = () => epoch === this.#epoch && current();
    if (!valid()) return {status: 'cancelled'};
    try {
      if (!this.#pending || this.#pending.version !== version) {
        const pending = {version, promise: this.#ports.read()};
        this.#pending = pending;
        const clear = () => {if (this.#pending === pending) this.#pending = undefined;};
        // Observe both outcomes: an ignored finally promise can itself reject.
        void pending.promise.then(clear, clear);
      }
      const result = await this.#pending.promise;
      if (!valid()) return {status: 'cancelled'};
      this.#revision = undefined;
      return this.#project(result);
    } catch {
      if (!valid()) return {status: 'cancelled'};
      this.#revision = undefined;
      return {status: 'error', code: 'bridge_failed'};
    }
  }
  async measure(current: () => boolean): Promise<UsageResult> {
    const epoch = this.#epoch;
    const valid = () => epoch === this.#epoch && current();
    if (!valid()) return {status: 'cancelled'};
    try {
      const result = await this.#ports.usage();
      return valid() ? result : {status: 'cancelled'};
    } catch { return valid() ? {status: 'error', code: 'bridge_failed'} : {status: 'cancelled'}; }
  }
  async mutate(operation: 'save' | 'reset', policy: unknown, current: () => boolean): Promise<NativeRetentionResult> {
    if (!['save', 'reset'].includes(operation)) return {status: 'error', code: 'invalid_request'};
    if (operation === 'save' && !isRetentionPolicy(policy)) throw Error('Invalid retention policy');
    if (!this.#revision) return {status: 'error', code: 'refresh_required'};
    const expected = this.#revision;
    this.#revision = undefined;
    const epoch = this.#epoch;
    const valid = () => epoch === this.#epoch && current();
    if (!valid()) return {status: 'cancelled'};
    try {
      const result = await this.#ports.write(operation, {expected, ...(operation === 'save' ? {policy: policy as RetentionPolicy} : {})});
      if (!valid()) return {status: 'cancelled'};
      const projected = this.#project(result);
      return result.status === 'connected' && projected.status === 'error'
        ? {status: 'error', code: 'outcome_unknown'} : projected;
    } catch {
      if (!valid()) return {status: 'cancelled'};
      this.#revision = undefined;
      return {status: 'error', code: 'outcome_unknown'};
    }
  }
}

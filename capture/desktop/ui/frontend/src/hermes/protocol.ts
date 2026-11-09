export const tokenValid = (value: unknown): boolean => typeof value === 'string' && /^[a-f0-9]{32}$/.test(value);
export const sourceValid = (value: unknown): value is string => typeof value === 'string' && value.length <= 4096 &&
  !/[\x00-\x1f\x7f]/.test(value) && (tokenValid(value) || (value.startsWith('/') && value.endsWith('.apparent')));
export type Receipt = { capture_id?: string; status?: string; bundle_path?: string; source?: string; error?: unknown };
export type View = { phase: 'confirm' | 'starting' | 'recording' | 'processing' | 'saved' | 'cancelled' | 'error'; message?: string; source?: string; code?: 'occupied' };
export function destination(request: string) {
  const [token, portText, ...rest] = request.split('/');
  const port = Number(portText);
  if (rest.length || !tokenValid(token) || !/^\d{5}$/.test(portText ?? '') || port < 32768 || port > 60999) throw new Error('Invalid Hermes destination');
  return { token, port };
}
export class Recording {
  readonly id: string;
  private run: (args: string[]) => Promise<Receipt>;
  constructor(id: string, run: (args: string[]) => Promise<Receipt>) {
    if (!tokenValid(id)) throw new Error('Invalid capture identity');
    this.id = id; this.run = run;
  }
  async start() {
    const receipt = await this.run(['capture', 'start', '--request-id', this.id, '--wait', '--timeout', '30']);
    if (receipt.capture_id === this.id && receipt.error === 'timeout') return receipt;
    return this.checked(receipt);
  }
  async inspect() { return this.checked(await this.run(['capture', 'inspect', this.id])); }
  async source() {
    const value = this.checked(await this.run(['capture', 'handoff', this.id]));
    if (!sourceValid(value.source)) throw new Error('Recording has no usable handoff');
    return value.source;
  }
  private checked(receipt: Receipt) {
    if (receipt.capture_id !== this.id || receipt.error) throw new Error('Unable to read this recording. Finish or cancel it in Apparent.');
    return receipt;
  }
}

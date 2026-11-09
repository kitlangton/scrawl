export interface Destination { session: string; connectionId: string; profile: string; targetProfile: string; mode: string; local: boolean }
export const validToken = (s: unknown): boolean => typeof s === 'string' && /^[a-f0-9]{32}$/.test(s);
export function launchURL(token: string, port: number) {
  if (!validToken(token) || !Number.isInteger(port) || port < 32768 || port > 60999) throw new Error('Invalid recording request');
  return `apparent://hermes/${token}/${port}`;
}
// Stock Desktop refuses custom schemes through PluginOs. The public session
// shell API can ask the local OS to open this one strictly validated URI.
export function launchCommand(token: string, port: number) {
  const url = launchURL(token, port);
  return `case "$(uname -s)" in Darwin) open '${url}';; Linux) xdg-open '${url}';; *) exit 64;; esac`;
}
export function keyCommand(token: string) {
  if (!validToken(token)) throw new Error('Invalid recording request');
  return `case "$(uname -s)" in Darwin) apparent_bridge_dir="$(getconf DARWIN_USER_TEMP_DIR)";; Linux) apparent_bridge_dir="\${XDG_RUNTIME_DIR:-/run/user/$(id -u)}";; *) exit 64;; esac\ncat "$apparent_bridge_dir/apparent-hermes/${token}.key"`;
}
export function validSource(s: unknown): s is string {
  return typeof s === 'string' && s.length <= 4096 && !/[\x00-\x1f\x7f]/.test(s) &&
    (validToken(s) || (s.startsWith('/') && s.endsWith('.apparent')));
}
export function submission(destination: Destination, token: string, source: string) {
  if (!destination.session || !destination.connectionId || !destination.profile || !validToken(token) || !validSource(source)) throw new Error('Invalid recording delivery');
  if (!destination.local && !validToken(source)) throw new Error('This recording is local. Enable Connect uploads in Apparent before recording for a remote Hermes backend. Nothing was sent.');
  return { session_id: destination.session, input_id: token,
    text: `Apparent recording (.apparent): ${source}\nUse apparent_inspect with this exact source to read my spoken instructions and the recording’s images, then act on them.` };
}
export interface Pending { version: 2; token: string; port: number; destination: Destination; runtime: Runtime; created: number; source?: string; sending?: boolean }
const hex = (bytes: ArrayBuffer) => Array.from(new Uint8Array(bytes), b => b.toString(16).padStart(2, '0')).join('');
export async function mailboxId(secret: string) {
  if (!validToken(secret)) throw new Error('Invalid mailbox capability');
  return hex(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(secret)));
}
export async function sign(secret: string, text: string) {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  return hex(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(text)));
}
export async function authenticatedState(secret: string, packet: unknown) {
  const value = packet as { payload?: unknown; mac?: unknown };
  if (!value || typeof value.payload !== 'string' || value.payload.length > 24576 || typeof value.mac !== 'string' || !/^[a-f0-9]{64}$/.test(value.mac)) throw new Error('Unauthenticated Apparent result');
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['verify']);
  const signature = Uint8Array.from(value.mac.match(/../g)!, pair => parseInt(pair, 16));
  if (!await crypto.subtle.verify('HMAC', key, signature, new TextEncoder().encode(value.payload))) throw new Error('Unauthenticated Apparent result');
  return JSON.parse(value.payload);
}
export interface Runtime { instance_id: string; profile_id: string; authority_epoch: number; capabilities: string[] }
export function runtime(value: Runtime): Runtime {
  if (!value || !value.instance_id || !value.profile_id || !Number.isInteger(value.authority_epoch) || !value.capabilities?.includes('durable-admission-v1')) throw new Error('Hermes gateway lacks the required identity/admission contract');
  return { instance_id: value.instance_id, profile_id: value.profile_id, authority_epoch: value.authority_epoch, capabilities: ['durable-admission-v1'] };
}
export function sameRuntime(a: Runtime, b: Runtime) { return a.instance_id === b.instance_id && a.profile_id === b.profile_id && a.authority_epoch === b.authority_epoch; }

import { RecipientTransportError } from "../recipient/transport.ts";
import { postAccountJson as post } from "./request.ts";
import { validatePeerAuthorityKey, type PeerAuthorityKey } from "../recipient/peer-authority.ts";

export type DeviceScope = "read" | "upload";
export type DeviceStartOptions = Readonly<{
  approvedServer: string; scope: DeviceScope; encryptionKey?: PeerAuthorityKey;
  timeoutMs?: number; signal?: AbortSignal; active?: () => boolean;
}>;
export type DevicePollOptions = Readonly<{
  approvedServer: string; deviceCode: string; timeoutMs?: number; signal?: AbortSignal; active?: () => boolean;
}>;

const fail = (category: ConstructorParameters<typeof RecipientTransportError>[0]): never => {
  throw new RecipientTransportError(category);
};
const validDeviceCode = (value: unknown): value is string => typeof value === "string" && value.length >= 1 && value.length <= 8192 && /^[\x21-\x7e]+$/.test(value);

/**
 * Anonymous, one-request device protocol transport. Replies are private untrusted
 * protocol data: authorizeDevice must validate all fields before displaying or
 * acting on them. This module creates no identity or transaction state, and never
 * persists grants. The caller's native single writer owns login incarnation,
 * cancellation, issue time, durable facts and commit. It must bind poll codes to
 * its started transaction and enforce consent before calling this transport.
 */
export function requestDeviceStart(options: DeviceStartOptions): Promise<unknown> {
  // Snapshot primitive values and validate/copy public key before asynchronous work.
  const approvedServer = options?.approvedServer;
  const scope = options?.scope;
  const timeoutMs = options?.timeoutMs;
  const signal = options?.signal;
  const active = options?.active;
  if (scope !== "read" && scope !== "upload") fail("invalid_request");
  let encryptionKey: PeerAuthorityKey | undefined;
  if (options?.encryptionKey !== undefined) {
    try { encryptionKey = validatePeerAuthorityKey(options.encryptionKey); }
    catch { fail("invalid_request"); }
  }
  const payload = Object.freeze(encryptionKey ? { scope, encryptionKey } : { scope });
  return post("/v1/device/start", approvedServer, payload, timeoutMs, signal, active);
}

/** One anonymous poll. The caller must bind deviceCode to its started transaction. */
export function requestDevicePoll(options: DevicePollOptions): Promise<unknown> {
  const approvedServer = options?.approvedServer;
  const deviceCode = options?.deviceCode;
  const timeoutMs = options?.timeoutMs;
  const signal = options?.signal;
  const active = options?.active;
  if (!validDeviceCode(deviceCode)) fail("invalid_request");
  const payload = Object.freeze({ deviceCode });
  return post("/v1/device/poll", approvedServer, payload, timeoutMs, signal, active);
}

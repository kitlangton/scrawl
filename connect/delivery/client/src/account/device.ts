import { decodeStrictJson } from "../recipient/json.ts";
import { normalizeRecipientServer, readBoundedTransportBody, RecipientTransportError } from "../recipient/transport.ts";
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
const validTimeout = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 1 && (value as number) <= 30_000;
const validDeviceCode = (value: unknown): value is string => typeof value === "string" && value.length >= 1 && value.length <= 8192 && /^[\x21-\x7e]+$/.test(value);

/**
 * Anonymous, one-request device protocol transport. Replies are private untrusted
 * protocol data: authorizeDevice must validate all fields before displaying or
 * acting on them. This module creates no identity or transaction state, and never
 * persists grants. The caller's native single writer owns login incarnation,
 * cancellation, issue time, durable facts and commit. It must bind poll codes to
 * its started transaction and enforce consent before calling this transport.
 */
async function post(path: "/v1/device/start" | "/v1/device/poll", serverInput: string, payload: unknown,
  timeoutInput: number | undefined, signal: AbortSignal | undefined, active: (() => boolean) | undefined): Promise<unknown> {
  // All options and request bytes are detached synchronously before the first await.
  const server = normalizeRecipientServer(serverInput);
  const timeout = timeoutInput ?? 30_000;
  if (!validTimeout(timeout)) fail("invalid_request");
  if (signal !== undefined && (signal === null || typeof signal !== "object" || typeof signal.aborted !== "boolean" || typeof signal.addEventListener !== "function" || typeof signal.removeEventListener !== "function")) fail("invalid_request");
  if (active !== undefined && typeof active !== "function") fail("invalid_request");
  const request = new TextEncoder().encode(JSON.stringify(payload));
  const controller = new AbortController();
  let responseBody: ReadableStream<Uint8Array> | null = null;
  let responseBytes: Uint8Array | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const callerSignal = signal;
  const abort = () => controller.abort("caller");
  const check = () => {
    if (controller.signal.aborted) fail(controller.signal.reason === "deadline" ? "timeout" : "cancelled");
    if (callerSignal?.aborted) fail("cancelled");
    if (active) {
      let current = false;
      try { current = active() === true; } catch { current = false; }
      if (!current) fail("cancelled");
    }
  };
  try {
    check();
    callerSignal?.addEventListener("abort", abort, { once: true });
    if (callerSignal?.aborted) abort();
    timer = setTimeout(() => controller.abort("deadline"), timeout);
    const response = await fetch(`${server}${path}`, {
      method: "POST", body: request.buffer,
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      credentials: "omit", redirect: "manual", referrerPolicy: "no-referrer", signal: controller.signal,
    });
    responseBody = response.body;
    check();
    responseBytes = await readBoundedTransportBody(responseBody, 2_000_000, controller.signal);
    responseBody = null;
    check();
    if (response.status >= 300 && response.status < 400) fail("redirect_refused");
    if (response.status === 429) fail("slow_down");
    if (!response.ok) {
      if (response.status === 401) fail("unauthorized");
      if (response.status === 403) fail("forbidden");
      if (response.status === 404) fail("not_found");
      fail("request_failed");
    }
    let result: unknown;
    try { result = decodeStrictJson(responseBytes, 2_000_000); }
    catch { fail("invalid_response"); }
    check();
    return result!;
  } catch (error) {
    if (controller.signal.aborted) fail(controller.signal.reason === "deadline" ? "timeout" : "cancelled");
    if (error instanceof RecipientTransportError) throw error;
    fail("request_failed");
  } finally {
    if (timer !== undefined) clearTimeout(timer);
    callerSignal?.removeEventListener("abort", abort);
    request.fill(0); responseBytes?.fill(0);
    if (responseBody && !responseBody.locked) { try { await responseBody.cancel(); } catch { /* category-only error surface */ } }
    controller.abort();
  }
}

/** One anonymous POST to the exact approved service base. */
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

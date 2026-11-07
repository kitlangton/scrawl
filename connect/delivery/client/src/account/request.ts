import { decodeStrictJson } from "../recipient/json.ts";
import { normalizeRecipientServer, readBoundedTransportBody, RecipientTransportError } from "../recipient/transport.ts";

const fail = (category: ConstructorParameters<typeof RecipientTransportError>[0]): never => { throw new RecipientTransportError(category); };
const validTimeout = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 1 && (value as number) <= 30_000;

/** Internal closed-route JSON POST mechanism. No retries, cookies or redirects.
 * Device routes are always anonymous; only the revocation route admits a bearer.
 * Returned parsed replies remain untrusted private data for the specific caller.
 */
export async function postAccountJson(path: "/v1/device/start" | "/v1/device/poll" | "/v1/token/revoke", serverInput: string, payload: unknown,
  timeoutInput: number | undefined, signal: AbortSignal | undefined, active: (() => boolean) | undefined, bearer?: string): Promise<unknown> {
  // All options and request bytes are detached synchronously before the first await.
  if (!["/v1/device/start", "/v1/device/poll", "/v1/token/revoke"].includes(path)) fail("invalid_request");
  if (bearer !== undefined && (path !== "/v1/token/revoke" || typeof bearer !== "string" || bearer.length < 1 || bearer.length > 8192 || !/^[\x21-\x7e]+$/.test(bearer))) fail("invalid_request");
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
      headers: { "Content-Type": "application/json", Accept: "application/json", ...(bearer === undefined ? {} : { Authorization: `Bearer ${bearer}` }) },
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

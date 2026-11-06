import { decodeStoredCredential, type StoredCredential } from "./wire.ts";
import { decodeStrictJson } from "../recipient/json.ts";
import { normalizeRecipientServer, readBoundedTransportBody, RecipientTransportError } from "../recipient/transport.ts";

const remoteCodes = ["file_too_large", "manifest_conflict", "beta_required", "slow_down", "quota_exceeded", "unauthorized", "forbidden", "expired", "rejected", "not_found", "conflict", "authorization_pending", "device_expired", "device_rejected", "reauthorization_required"] as const;
export type RenewalRequestFailure = typeof remoteCodes[number] | "auth_required" | "retry_later" | "redirect_refused" | "invalid_request"
  | "invalid_server" | "response_too_large" | "cancelled" | "timeout" | "request_failed" | "http_error";
export class RenewalRequestError extends Error {
  readonly category: RenewalRequestFailure;
  constructor(category: RenewalRequestFailure) { super(category); this.name = "RenewalRequestError"; this.category = category; }
}
const fail = (category: RenewalRequestFailure): never => { throw new RenewalRequestError(category); };

/** Single refresh POST, with no retries, persistence, rejection marking or bearer header.
 * Caller must first durably prepare the legacy journal under its sole-writer locks.
 * Returned raw bytes belong to the caller: persist before marking completion and
 * parsing/merging with the journal's original issue time, then wipe owned bytes.
 * Any failure may follow a server-side rotation; it never authorizes a blind retry.
 */
export async function requestRenewalReply(credential: StoredCredential,
  options: { approvedServer: string; timeoutMs?: number; signal?: AbortSignal }): Promise<Uint8Array> {
  let body: ReadableStream<Uint8Array> | null = null, request: Uint8Array<ArrayBuffer> | undefined, responseBytes: Uint8Array | undefined;
  const controller = new AbortController(); let timer: ReturnType<typeof setTimeout> | undefined;
  let signal: AbortSignal | undefined;
  const abort = () => controller.abort("caller");
  try {
    const approvedServer = options.approvedServer, timeout = options.timeoutMs ?? 30_000; signal = options.signal;
    if (signal?.aborted) fail("cancelled");
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 30_000) fail("invalid_request");
    const server = normalizeRecipientServer(approvedServer);
    if (credential.server !== approvedServer) fail("invalid_server");
    const bytes = Buffer.from(JSON.stringify(credential));
    let current: StoredCredential;
    try { current = decodeStoredCredential(bytes, credential.scope, approvedServer); } catch { return fail("invalid_request"); }
    finally { bytes.fill(0); }
    if ((current.scope !== "read" && current.scope !== "upload") || current.authRequired || !current.refreshToken) fail("invalid_request");
    request = new TextEncoder().encode(JSON.stringify({ refreshToken: current.refreshToken }));
    signal?.addEventListener("abort", abort, { once: true });
    timer = setTimeout(() => controller.abort("deadline"), timeout);
    if (signal?.aborted) abort();
    const response = await fetch(`${server}/v1/token/refresh`, { method: "POST", body: request.buffer,
      headers: { "Content-Type": "application/json", Accept: "application/json" }, credentials: "omit",
      redirect: "manual", referrerPolicy: "no-referrer", signal: controller.signal });
    body = response.body;
    responseBytes = await readBoundedTransportBody(body, 2_000_000, controller.signal);
    if (response.status >= 300 && response.status < 400) fail("redirect_refused");
    if (response.status === 401) fail("auth_required");
    if (response.status === 429 || response.status >= 500) fail("retry_later");
    if (!response.ok) {
      let code: unknown;
      try {
        const parsed = decodeStrictJson(responseBytes, 2_000_000) as { error?: { code?: unknown } } | null;
        code = parsed?.error?.code;
      } catch { /* Remote text is never exposed. */ }
      fail(typeof code === "string" && remoteCodes.includes(code as typeof remoteCodes[number]) ? code as typeof remoteCodes[number] : "http_error");
    }
    const output = responseBytes; responseBytes = undefined; return output;
  } catch (error) {
    if (controller.signal.aborted) fail(controller.signal.reason === "deadline" ? "timeout" : "cancelled");
    if (error instanceof RenewalRequestError) throw error;
    if (error instanceof RecipientTransportError && ["invalid_server", "invalid_request", "response_too_large", "cancelled", "timeout"].includes(error.category)) fail(error.category as RenewalRequestFailure);
    return fail("request_failed");
  } finally {
    if (timer !== undefined) clearTimeout(timer);
    signal?.removeEventListener("abort", abort);
    request?.fill(0); responseBytes?.fill(0);
    if (body && !body.locked) { try { await body.cancel(); } catch {} }
    controller.abort();
  }
}

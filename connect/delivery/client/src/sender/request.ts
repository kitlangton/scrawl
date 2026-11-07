import { decodeStrictJson } from "../recipient/json.ts";
import { normalizeRecipientServer, readBoundedTransportBody, RecipientTransportError } from "../recipient/transport.ts";

export type Authorization = { approvedServer: string; scope: "upload"; token: string };
export type ScopedAuthorization = { approvedServer: string; scope: "read" | "upload"; token: string };
export type RequestOptions = { server: string; signal?: AbortSignal; active(): boolean; authorize(server: string, scope: "upload", signal: AbortSignal): Promise<Authorization> };
export type ScopedRequestOptions = { server: string; scope: "read" | "upload"; signal?: AbortSignal; active(): boolean; authorize(server: string, scope: "read" | "upload", signal: AbortSignal): Promise<ScopedAuthorization> };
const JSON_LIMIT = 2_000_000, TOKEN_LIMIT = 8192;
export const fail = (category: ConstructorParameters<typeof RecipientTransportError>[0]): never => { throw new RecipientTransportError(category); };
export function checkActive(options: Pick<ScopedRequestOptions, "active">, signal: AbortSignal): void {
  if (signal.aborted) fail(signal.reason === "deadline" ? "timeout" : "cancelled");
  try { if (!options.active()) fail("stale_snapshot"); } catch (error) { if (error instanceof RecipientTransportError) throw error; fail("stale_snapshot"); }
  if (signal.aborted) fail(signal.reason === "deadline" ? "timeout" : "cancelled");
}
export async function readPreparedAsset(options: RequestOptions & { readAsset(path: string, signal: AbortSignal): Promise<Uint8Array> }, path: string): Promise<Uint8Array> {
  const controller = new AbortController(), abortCaller = () => controller.abort("caller");
  if (options.signal?.aborted) fail("cancelled"); options.signal?.addEventListener("abort", abortCaller, { once: true });
  const timeout = setTimeout(() => controller.abort("deadline"), 60_000); let onAbort: (() => void) | undefined;
  try { checkActive(options, controller.signal); const aborted = new Promise<never>((_r, reject) => { onAbort = () => reject(new RecipientTransportError(controller.signal.reason === "deadline" ? "timeout" : "cancelled")); controller.signal.addEventListener("abort", onAbort, { once: true }); });
    return await Promise.race([options.readAsset(path, controller.signal), aborted]);
  } finally { clearTimeout(timeout); options.signal?.removeEventListener("abort", abortCaller); if (onAbort) controller.signal.removeEventListener("abort", onAbort); controller.abort(); }
}
export async function scopedRequest(options: ScopedRequestOptions, server: string, route: string, method: "GET" | "DELETE", payload?: Uint8Array): Promise<Uint8Array> {
  const pinned = Object.freeze({ server, scope: options.scope, active: options.active, authorize: options.authorize, signal: options.signal });
  return requestCore(pinned, server, route, method, payload, true, 30_000);
}
export async function request(options: RequestOptions, server: string, route: string, method: "POST" | "PUT" | "GET", payload: Uint8Array | undefined, json: boolean): Promise<Uint8Array> {
  const pinned = Object.freeze({ server, scope: "upload" as const, active: options.active, authorize: options.authorize, signal: options.signal });
  return requestCore(pinned, server, route, method, payload, json, method === "PUT" ? 60_000 : 30_000);
}
async function requestCore(options: ScopedRequestOptions, server: string, route: string, method: "POST" | "PUT" | "GET" | "DELETE", payload: Uint8Array | undefined, json: boolean, timeoutMs: number): Promise<Uint8Array> {
  const controller = new AbortController(), abortCaller = () => controller.abort("caller");
  if (options.signal?.aborted) fail("cancelled"); options.signal?.addEventListener("abort", abortCaller, { once: true });
  const timeout = setTimeout(() => controller.abort("deadline"), timeoutMs); let body: ReadableStream<Uint8Array> | null = null;
  try { checkActive(options, controller.signal); let onAbort!: () => void;
    const aborted = new Promise<never>((_r, reject) => { onAbort = () => reject(new RecipientTransportError(controller.signal.reason === "deadline" ? "timeout" : "cancelled")); controller.signal.addEventListener("abort", onAbort, { once: true }); });
    const scope = options.scope;
    let auth: ScopedAuthorization; try { auth = await Promise.race([options.authorize(server, scope, controller.signal), aborted]); } finally { controller.signal.removeEventListener("abort", onAbort); }
    checkActive(options, controller.signal); if (!auth || auth.approvedServer !== server || auth.scope !== scope) fail("auth_required");
    if (typeof auth.token !== "string" || auth.token.length === 0 || auth.token.length > TOKEN_LIMIT || !/^[\x21-\x7e]+$/.test(auth.token)) fail("auth_required");
    const response = await fetch(`${server}${route}`, { method, headers: { Authorization: `Bearer ${auth.token}`, Accept: json ? "application/json" : "application/octet-stream", ...(payload ? { "Content-Type": method === "PUT" ? "application/octet-stream" : "application/json" } : {}) }, ...(payload ? { body: payload.buffer as ArrayBuffer } : {}), redirect: "manual", credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal });
    body = response.body; checkActive(options, controller.signal); if (response.status >= 300 && response.status < 400) fail("redirect_refused");
    if (!response.ok) { if (response.status === 401) fail("unauthorized"); if (response.status === 403) fail("forbidden"); fail("request_failed"); }
    const bytes = await readBoundedTransportBody(body, json ? JSON_LIMIT : 128 * 1024, controller.signal); checkActive(options, controller.signal); return bytes;
  } catch (cause) { if (cause instanceof RecipientTransportError) throw cause; if (controller.signal.aborted) return fail(controller.signal.reason === "deadline" ? "timeout" : "cancelled"); return fail("request_failed"); }
  finally { clearTimeout(timeout); options.signal?.removeEventListener("abort", abortCaller); if (body && !body.locked) { try { await body.cancel(); } catch {} } controller.abort(); }
}
export function normalizeServer(server: string): string { try { return normalizeRecipientServer(server); } catch { return fail("invalid_server"); } }
export function json(bytes: Uint8Array): Record<string, unknown> { let value: unknown; try { value = decodeStrictJson(bytes, JSON_LIMIT); } catch { fail("invalid_response"); } if (!value || typeof value !== "object" || Array.isArray(value)) fail("invalid_response"); return value as Record<string, unknown>; }

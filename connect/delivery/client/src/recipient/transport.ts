import { decodeStrictJson } from "./json.ts";
import { assetPath } from "./manifest.ts";

export type RecipientTransportCategory = "not_found" | "unauthorized" | "forbidden" | "slow_down" | "quota_exceeded"
  | "request_failed" | "response_too_large" | "invalid_response" | "invalid_server" | "invalid_request"
  | "redirect_refused" | "cancelled" | "timeout";

/** Safe transport failure. The message and category never include response or request data. */
export class RecipientTransportError extends Error {
  readonly category: RecipientTransportCategory;
  constructor(category: RecipientTransportCategory) {
    super(category);
    this.name = "RecipientTransportError";
    this.category = category;
  }
}

const JSON_MAX = 2_000_000;
const ENVELOPE_MAX = 128 * 1024;
const SEALED_MAX = 32 * 1024 * 1024;
const FILE_MAX = 90_000_000;
const TOKEN_MAX = 8192;
const TRANSPORT = /^\/v1\/captures\/([a-f0-9]{32})\/files\/(.+)$/;

function error(category: RecipientTransportCategory): never { throw new RecipientTransportError(category); }

function validateServer(server: string): string {
  if (typeof server !== "string" || server.length > 2048 || /[^\x21-\x7e]|["'\\?#@]/.test(server)) error("invalid_server");
  const base = server.replace(/\/+$/, "");
  if (!/^https?:\/\//.test(base)) error("invalid_server");
  const [authority, ...path] = base.replace(/^https?:\/\//, "").split("/");
  if (!authority || !/^[A-Za-z0-9.-]+(?::[0-9]+)?$/.test(authority)
    || path.length > 1 || path.length === 1 && path[0] !== "connect") error("invalid_server");
  let url: URL;
  try { url = new URL(base); } catch { error("invalid_server"); }
  const local = url!.protocol === "http:" && ["localhost", "127.0.0.1"].includes(authority!.split(":")[0]!);
  if (url!.protocol !== "https:" && !local) error("invalid_server");
  if (url!.username || url!.password || url!.search || url!.hash) error("invalid_server");
  if (url!.pathname !== "/" && url!.pathname !== "/connect") error("invalid_server");
  // Preserve the exact approved transport identity used in signed contexts.
  return base;
}

function validateToken(token: string): void {
  if (typeof token !== "string" || token.length === 0 || token.length > TOKEN_MAX || !/^[\x21-\x7e]+$/.test(token)) error("invalid_request");
}

function routeLimit(route: string, kind: "json" | "bytes"): number {
  if (route === "/v1/account" || route === "/v1/encryption/peers" || /^\/v1\/captures\/[a-f0-9]{32}$/.test(route)) {
    if (kind !== "json") error("invalid_request");
    return JSON_MAX;
  }
  const match = TRANSPORT.exec(route);
  if (!match) error("invalid_request");
  if (kind !== "bytes") error("invalid_request");
  const name = match![2]!;
  const transportName = name.replace(/^(evidence|media)\//, "");
  if (transportName === "envelope.json") return ENVELOPE_MAX;
  const part = /^sealed\/part-([0-9]{6})\.age$/.exec(transportName);
  if (part && Number(part[1]) < 16) return SEALED_MAX;
  if (!assetPath(name)) error("invalid_request");
  return name === "apparent.json" ? JSON_MAX : FILE_MAX;
}

export class RecipientTransport {
  #server: string;
  #accessToken: string;
  #signal?: AbortSignal;
  #timeoutMs?: number;
  get server(): string { return this.#server; }

  constructor(server: string, accessToken: string, options?: { signal?: AbortSignal; timeoutMs?: number }) {
    this.#server = validateServer(server);
    validateToken(accessToken);
    if (options?.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 60_000)) error("invalid_request");
    this.#accessToken = accessToken;
    this.#signal = options?.signal;
    this.#timeoutMs = options?.timeoutMs;
  }

  getJson(route: string, maximumBytes = JSON_MAX): Promise<unknown> {
    const hard = routeLimit(route, "json");
    if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 0 || maximumBytes > hard) error("invalid_request");
    return this.request(route, maximumBytes, 30_000, true).then(bytes => {
      try { return decodeStrictJson(bytes, maximumBytes); } catch { error("invalid_response"); }
    });
  }

  getBytes(route: string, maximumBytes: number): Promise<Uint8Array> {
    const hard = routeLimit(route, "bytes");
    if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 0 || maximumBytes > hard) error("invalid_request");
    const timeout = this.#timeoutMs ?? 60_000;
    return this.request(route, maximumBytes, timeout, false);
  }

  private async request(route: string, maximumBytes: number, defaultTimeout: number, json: boolean): Promise<Uint8Array> {
    const controller = new AbortController();
    const abortFromCaller = () => controller.abort("caller");
    if (this.#signal?.aborted) error("cancelled");
    this.#signal?.addEventListener("abort", abortFromCaller, { once: true });
    const timer = setTimeout(() => controller.abort("deadline"), this.#timeoutMs ?? defaultTimeout);
    let body: ReadableStream<Uint8Array> | null = null;
    try {
      const response = await fetch(`${this.#server}${route}`, {
        method: "GET", headers: { Authorization: `Bearer ${this.#accessToken}`, Accept: json ? "application/json" : "application/octet-stream" },
        redirect: "manual", credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal,
      });
      body = response.body;
      if (response.status >= 300 && response.status < 400) error("redirect_refused");
      if (!response.ok) {
        let category: RecipientTransportCategory = "request_failed";
        if (response.status === 404) category = "not_found";
        else if (response.status === 401) category = "unauthorized";
        else if (response.status === 403) category = "forbidden";
        else if (response.status === 429) category = "slow_down";
        else if (response.status === 413 || response.status === 507) category = "quota_exceeded";
        error(category);
      }
      if (!body) return new Uint8Array();
      const reader = body.getReader();
      const chunks: Uint8Array[] = [];
      let total = 0;
      try {
        while (true) {
          const item = await reader.read();
          if (controller.signal.aborted) error(controller.signal.reason === "deadline" ? "timeout" : "cancelled");
          if (item.done) break;
          const chunk = item.value;
          if (chunk.byteLength > maximumBytes - total) error("response_too_large");
          total += chunk.byteLength;
          chunks.push(chunk);
        }
      } finally { reader.releaseLock(); }
      const output = new Uint8Array(total);
      let offset = 0;
      for (const chunk of chunks) { output.set(chunk, offset); offset += chunk.byteLength; }
      return output;
    } catch (cause) {
      if (cause instanceof RecipientTransportError) throw cause;
      if (controller.signal.aborted) error(controller.signal.reason === "deadline" ? "timeout" : "cancelled");
      return error("request_failed");
    } finally {
      clearTimeout(timer);
      this.#signal?.removeEventListener("abort", abortFromCaller);
      if (body && !body.locked) { try { await body.cancel(); } catch { /* category-only error surface */ } }
      controller.abort();
    }
  }
}

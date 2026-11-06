import { createHash } from "node:crypto";
import { assetPath } from "../recipient/manifest.ts";
import { decodeStrictJson } from "../recipient/json.ts";
import {
  normalizeRecipientServer, readBoundedTransportBody, RecipientTransportError,
} from "../recipient/transport.ts";
import type { Asset } from "../recipient/crypto.ts";

export type PreparedPlainAsset = Asset;
export type PlainUploadAuthorization = { approvedServer: string; scope: "upload"; token: string };
export type PlainUploadOptions = {
  server: string;
  captureId: string;
  files: readonly PreparedPlainAsset[];
  /** Reads the caller-owned stable snapshot. Returned bytes remain caller-owned. */
  readAsset(path: string, signal: AbortSignal): Promise<Uint8Array>;
  /** Reauthorize each request for this exact server and upload scope. */
  authorize(server: string, scope: "upload", signal: AbortSignal): Promise<PlainUploadAuthorization>;
  /** Caller freshness/cancellation guard, checked at every request boundary. */
  active?(): boolean;
  signal?: AbortSignal;
};

const FILE_LIMIT = 90_000_000;
const TOTAL_LIMIT = 500_000_000;
const MANIFEST_LIMIT = 128 * 1024;
const JSON_LIMIT = 2_000_000;
const TOKEN_LIMIT = 8192;
const ID = /^[a-f0-9]{32}$/;
const HASH = /^[a-f0-9]{64}$/;
const fail = (category: ConstructorParameters<typeof RecipientTransportError>[0]): never => {
  throw new RecipientTransportError(category);
};

function validateFiles(input: readonly PreparedPlainAsset[]): PreparedPlainAsset[] {
  if (!Array.isArray(input) || input.length === 0 || input.length > 512) fail("invalid_request");
  const names = new Set<string>();
  let total = 0;
  const files = input.map(file => {
    if (!file || typeof file !== "object" || !assetPath(file.path) || names.has(file.path)
      || !Number.isSafeInteger(file.size) || file.size < 0 || file.size > FILE_LIMIT
      || typeof file.sha256 !== "string" || !HASH.test(file.sha256)) fail("invalid_request");
    names.add(file.path); total += file.size;
    if (total > TOTAL_LIMIT || file.path === "apparent.json" && file.size > JSON_LIMIT) fail("invalid_request");
    return { path: file.path, size: file.size, sha256: file.sha256 };
  });
  for (const required of ["apparent.json", "summary.md", "contact-sheet.png"]) if (!names.has(required)) fail("invalid_request");
  if (new TextEncoder().encode(JSON.stringify({ files })).byteLength > MANIFEST_LIMIT) fail("invalid_request");
  return files;
}

function checkActive(options: PlainUploadOptions, signal: AbortSignal): void {
  if (signal.aborted) fail(signal.reason === "deadline" ? "timeout" : "cancelled");
  try { if (options.active && !options.active()) fail("stale_snapshot"); }
  catch (error) { if (error instanceof RecipientTransportError) throw error; fail("stale_snapshot"); }
}

function token(value: unknown): asserts value is string {
  if (typeof value !== "string" || value.length === 0 || value.length > TOKEN_LIMIT || !/^[\x21-\x7e]+$/.test(value)) fail("auth_required");
}

async function request(options: PlainUploadOptions, server: string, route: string, method: "POST" | "PUT" | "GET",
  payload: Uint8Array | undefined, json: boolean): Promise<Uint8Array> {
  const controller = new AbortController();
  const abortCaller = () => controller.abort("caller");
  if (options.signal?.aborted) fail("cancelled");
  options.signal?.addEventListener("abort", abortCaller, { once: true });
  const timeout = setTimeout(() => controller.abort("deadline"), method === "PUT" ? 60_000 : 30_000);
  let body: ReadableStream<Uint8Array> | null = null;
  try {
    checkActive(options, controller.signal);
    let onAbort!: () => void;
    const aborted = new Promise<never>((_resolve, reject) => {
      onAbort = () => reject(new RecipientTransportError(controller.signal.reason === "deadline" ? "timeout" : "cancelled"));
      controller.signal.addEventListener("abort", onAbort, { once: true });
    });
    let authorization: PlainUploadAuthorization;
    try { authorization = await Promise.race([options.authorize(server, "upload", controller.signal), aborted]); }
    finally { controller.signal.removeEventListener("abort", onAbort); }
    checkActive(options, controller.signal);
    if (!authorization || authorization.approvedServer !== server || authorization.scope !== "upload") fail("auth_required");
    token(authorization.token);
    const response = await fetch(`${server}${route}`, {
      method, headers: {
        Authorization: `Bearer ${authorization.token}`,
        Accept: json ? "application/json" : "application/octet-stream",
        ...(payload ? { "Content-Type": method === "PUT" ? "application/octet-stream" : "application/json" } : {}),
      },
      ...(payload ? { body: payload.buffer as ArrayBuffer } : {}),
      redirect: "manual", credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal,
    });
    body = response.body;
    checkActive(options, controller.signal);
    if (response.status >= 300 && response.status < 400) fail("redirect_refused");
    if (!response.ok) {
      if (response.status === 401) fail("unauthorized");
      if (response.status === 403) fail("forbidden");
      fail("request_failed");
    }
    const bytes = await readBoundedTransportBody(body, json ? JSON_LIMIT : 128 * 1024, controller.signal);
    checkActive(options, controller.signal);
    return bytes;
  } catch (cause) {
    if (cause instanceof RecipientTransportError) throw cause;
    if (controller.signal.aborted) return fail(controller.signal.reason === "deadline" ? "timeout" : "cancelled");
    return fail("request_failed");
  } finally {
    clearTimeout(timeout);
    options.signal?.removeEventListener("abort", abortCaller);
    if (body && !body.locked) { try { await body.cancel(); } catch { /* category-only failure surface */ } }
    controller.abort();
  }
}

function json(bytes: Uint8Array): Record<string, unknown> {
  let value: unknown;
  try { value = decodeStrictJson(bytes, JSON_LIMIT); } catch { fail("invalid_response"); }
  if (!value || typeof value !== "object" || Array.isArray(value)) fail("invalid_response");
  return value as Record<string, unknown>;
}

function responseFiles(value: unknown): PreparedPlainAsset[] {
  if (!Array.isArray(value) || value.length > 512) fail("invalid_response");
  const files: PreparedPlainAsset[] = [];
  const names = new Set<string>();
  for (const raw of value as unknown[]) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) fail("invalid_response");
    const row = raw as Record<string, unknown>;
    if (Object.keys(row).sort().join(",") !== "path,sha256,size" || !assetPath(row.path) || names.has(row.path as string)
      || !Number.isSafeInteger(row.size) || (row.size as number) < 0 || (row.size as number) > FILE_LIMIT
      || typeof row.sha256 !== "string" || !HASH.test(row.sha256)) fail("invalid_response");
    names.add(row.path as string);
    files.push({ path: row.path as string, size: row.size as number, sha256: row.sha256 as string });
  }
  return files;
}

function sameManifest(left: readonly PreparedPlainAsset[], right: readonly PreparedPlainAsset[]): boolean {
  if (left.length !== right.length) return false;
  const expected = new Map(left.map(file => [file.path, file]));
  return right.every(file => {
    const match = expected.get(file.path);
    return match !== undefined && match.size === file.size && match.sha256 === file.sha256;
  });
}

/** Upload an already stable, consented plaintext snapshot. This never encrypts or falls back from encryption. */
export async function uploadPreparedPlainCapture(options: PlainUploadOptions): Promise<void> {
  const server = normalizeRecipientServer(options.server);
  if (typeof options.captureId !== "string" || !ID.test(options.captureId)) fail("invalid_request");
  if (typeof options.readAsset !== "function" || typeof options.authorize !== "function") fail("invalid_request");
  const files = validateFiles(options.files);
  const base = `/v1/captures/${options.captureId}`;
  const reservation = json(await request(options, server, base, "POST", new TextEncoder().encode(JSON.stringify({ files })), true));
  if (reservation.id !== options.captureId || reservation.status !== "pending" && reservation.status !== "ready") fail("invalid_response");
  if (reservation.status === "pending") {
    for (const file of files) {
      checkActive(options, options.signal ?? new AbortController().signal);
      const source = await options.readAsset(file.path, options.signal ?? new AbortController().signal);
      if (!(source instanceof Uint8Array)) fail("stale_snapshot");
      const owned = new Uint8Array(source);
      try {
        const sha = createHash("sha256").update(owned).digest("hex");
        if (owned.byteLength !== file.size || sha !== file.sha256) fail("stale_snapshot");
        await request(options, server, `${base}/files/${file.path}`, "PUT", owned, false);
      } finally { owned.fill(0); }
    }
    const completed = json(await request(options, server, `${base}/complete`, "POST", undefined, true));
    if (completed.id !== options.captureId || completed.status !== "ready") fail("invalid_response");
  }
  const verified = json(await request(options, server, base, "GET", undefined, true));
  if (verified.id !== options.captureId || verified.status !== "ready" || !sameManifest(files, responseFiles(verified.files))) fail("invalid_response");
}

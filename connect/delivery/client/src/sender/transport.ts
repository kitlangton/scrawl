import { createHash } from "node:crypto";
import { recipientAssets } from "../recipient/manifest.ts";
import type { Asset } from "../recipient/crypto.ts";
import { RecipientTransportError } from "../recipient/transport.ts";
import { checkActive, fail, json, normalizeServer, readPreparedAsset, request } from "./request.ts";

export type PreparedPlainAsset = Asset;
export type PlainUploadAuthorization = { approvedServer: string; scope: "upload"; token: string };
export type PlainUploadOptions = { server: string; captureId: string; files: readonly PreparedPlainAsset[]; readAsset(path: string, signal: AbortSignal): Promise<Uint8Array>; authorize(server: string, scope: "upload", signal: AbortSignal): Promise<PlainUploadAuthorization>; active(): boolean; signal?: AbortSignal };
const MANIFEST_LIMIT = 128 * 1024, ID = /^[a-f0-9]{32}$/;
const SAFE_FAILURES = new Set(["invalid_request", "invalid_response", "invalid_server", "stale_snapshot", "auth_required", "unauthorized", "forbidden", "redirect_refused", "cancelled", "timeout", "response_too_large", "request_failed"]);
function validateFiles(input: readonly PreparedPlainAsset[]): PreparedPlainAsset[] { let files: PreparedPlainAsset[]; try { files = recipientAssets(input); } catch { return fail("invalid_request"); } if (new TextEncoder().encode(JSON.stringify({ files })).byteLength > MANIFEST_LIMIT) fail("invalid_request"); return files; }
function sameManifest(a: readonly PreparedPlainAsset[], b: readonly PreparedPlainAsset[]): boolean { if (a.length !== b.length) return false; const m = new Map(a.map(x => [x.path, x])); return b.every(x => { const y = m.get(x.path); return !!y && y.size === x.size && y.sha256 === x.sha256; }); }
export async function uploadPreparedPlainCapture(input: PlainUploadOptions): Promise<void> {
  try { const options = { server: input.server, captureId: input.captureId, files: input.files, readAsset: input.readAsset, authorize: input.authorize, active: input.active, signal: input.signal };
    const server = normalizeServer(options.server); if (typeof options.captureId !== "string" || !ID.test(options.captureId) || typeof options.readAsset !== "function" || typeof options.authorize !== "function" || typeof options.active !== "function") fail("invalid_request");
    const files = validateFiles(options.files), base = `/v1/captures/${options.captureId}`;
    const reservation = json(await request(options, server, base, "POST", new TextEncoder().encode(JSON.stringify({ files })), true));
    if (reservation.id !== options.captureId || (reservation.status !== "pending" && reservation.status !== "ready")) fail("invalid_response");
    if (reservation.status === "pending") { for (const file of files) { checkActive(options, options.signal ?? new AbortController().signal); const source = await readPreparedAsset(options, file.path); if (!(source instanceof Uint8Array) || source.byteLength !== file.size) fail("stale_snapshot"); const owned = new Uint8Array(source); try { if (createHash("sha256").update(owned).digest("hex") !== file.sha256) fail("stale_snapshot"); await request(options, server, `${base}/files/${file.path}`, "PUT", owned, false); } finally { owned.fill(0); } }
      const completed = json(await request(options, server, `${base}/complete`, "POST", undefined, true)); if (completed.id !== options.captureId || completed.status !== "ready") fail("invalid_response"); }
    const verified = json(await request(options, server, base, "GET", undefined, true)); let received: PreparedPlainAsset[]; try { received = recipientAssets(verified.files); } catch { return fail("invalid_response"); }
    if (verified.id !== options.captureId || verified.status !== "ready" || !sameManifest(files, received)) fail("invalid_response");
  } catch (cause) { if (cause instanceof RecipientTransportError && SAFE_FAILURES.has(cause.category)) fail(cause.category); fail("request_failed"); }
}

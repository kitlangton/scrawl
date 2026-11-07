import { decodeStrictJson } from "../recipient/json.ts";
import { RecipientTransportError, type RecipientTransportCategory } from "../recipient/transport.ts";
import { scopedRequest, checkActive } from "../sender/request.ts";
import { pinAccountRequest, safeTransportCategory, type ShareOptions, type ShareAuthorization } from "./authority.ts";
export type { ShareOptions, ShareAuthorization } from "./authority.ts";

const ID = /^[a-f0-9]{32}$/;
const STATES = new Set(["pending", "ready", "paused", "failed", "expired", "revoked", "deleted"]);
const MAX_ROWS = 10_000, MAX_PAGES = 1_000, MAX_BYTES = 16_000_000;
export type Share = Readonly<{ id: string; captureId: string; title: string; status: "pending" | "ready" | "paused" | "failed" | "expired" | "revoked" | "deleted"; createdAt: string; expiresAt: string; url: string | null }>;
export type ShareList = Readonly<{ shares: readonly Share[]; nextCursor: null }>;
/** Lists are all-or-error: at most 1,000 pages, 10,000 rows and 16 MB raw response bytes; never truncate a management result. */
const snapshot = pinAccountRequest;
function invalid(): never { throw new RecipientTransportError("invalid_response"); }
function object(value: unknown): Record<string, unknown> { if (!value || typeof value !== "object" || Array.isArray(value)) return invalid(); return value as Record<string, unknown>; }
function timestamp(value: unknown): string {
  if (typeof value !== "string" || value.length > 40 || !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,3})?Z$/.test(value) || !Number.isFinite(Date.parse(value))) return invalid();
  return value;
}
function parseShare(value: unknown, server: string): Share {
  const row = object(value), fields = ["id", "captureId", "title", "status", "createdAt", "expiresAt", "url"];
  if (fields.some(field => !Object.hasOwn(row, field))) return invalid();
  if (typeof row.id !== "string" || !ID.test(row.id) || typeof row.captureId !== "string" || !ID.test(row.captureId)
      || typeof row.title !== "string" || row.title.length > 512 || typeof row.status !== "string" || !STATES.has(row.status)) return invalid();
  const status = row.status as Share["status"];
  let url: string | null;
  if (["revoked", "expired", "deleted"].includes(status)) { if (row.url !== null) return invalid(); url = null; }
  else {
    if (typeof row.url !== "string" || row.url.length > 4096) return invalid();
    const match = /\/s\/([a-f0-9]{64})$/.exec(row.url);
    if (!match || row.url !== `${server}/s/${match[1]}`) return invalid();
    url = row.url;
  }
  return Object.freeze({ id: row.id, captureId: row.captureId, title: row.title, status, createdAt: timestamp(row.createdAt), expiresAt: timestamp(row.expiresAt), url });
}
function parsePage(bytes: Uint8Array, server: string): { shares: Share[]; nextCursor: string | null } {
  let parsed: unknown; try { parsed = decodeStrictJson(bytes, 2_000_000); } catch { return invalid(); }
  const page = object(parsed);
  if (!Object.hasOwn(page, "shares") || !Object.hasOwn(page, "nextCursor") || !Array.isArray(page.shares) || page.shares.length > 100) return invalid();
  const shares = page.shares.map(row => parseShare(row, server));
  const cursor = page.nextCursor;
  if (cursor !== null && (typeof cursor !== "string" || cursor.length > 64 || !/^\d{1,16}:[a-f0-9]{32}$/.test(cursor) || !Number.isSafeInteger(Number(cursor.split(":")[0])))) return invalid();
  return { shares, nextCursor: cursor as string | null };
}
function encodeCursor(cursor: string): string { return Array.from(new TextEncoder().encode(cursor), byte => byte >= 65 && byte <= 90 || byte >= 97 && byte <= 122 || byte >= 48 && byte <= 57 || "-._~".includes(String.fromCharCode(byte)) ? String.fromCharCode(byte) : `%${byte.toString(16).toUpperCase().padStart(2, "0")}`).join(""); }
async function listSharePages(pinned: ReturnType<typeof snapshot>): Promise<ShareList> {
  const signal = pinned.signal ?? new AbortController().signal;
  const all: Share[] = [], ids = new Set<string>(), cursors = new Set<string>();
  let cursor: string | null = null, projected = 0;
  for (let pageNo = 0; pageNo < MAX_PAGES; pageNo++) {
    const route = `/v1/shares${cursor === null ? "" : `?before=${encodeCursor(cursor)}`}`;
    const bytes = await scopedRequest(pinned, pinned.server, route, "GET");
    let page: ReturnType<typeof parsePage>;
    try { projected += bytes.byteLength; if (projected > MAX_BYTES) throw new RecipientTransportError("response_too_large");
      page = parsePage(bytes, pinned.server); } finally { bytes.fill(0); }
    checkActive(pinned, signal);
    if (all.length + page.shares.length > MAX_ROWS) throw new RecipientTransportError("response_too_large");
    for (const row of page.shares) { if (ids.has(row.id)) invalid(); ids.add(row.id); all.push(row); }
    if (page.nextCursor === null) return Object.freeze({ shares: Object.freeze(all.slice()), nextCursor: null });
    if (cursors.has(page.nextCursor)) invalid(); cursors.add(page.nextCursor); cursor = page.nextCursor;
  }
  throw new RecipientTransportError("response_too_large");
}

export async function listShares(options: ShareOptions): Promise<ShareList> {
  try { return await listSharePages(snapshot(options)); }
  catch (error) { throw new RecipientTransportError(safeTransportCategory(error instanceof RecipientTransportError ? error.category : undefined)); }
}

export class ShareRevocationError extends Error {
  readonly effect: "unknown" | "acknowledged_unverified";
  readonly category: RecipientTransportCategory | "revocation_unverified";
  constructor(effect: "unknown" | "acknowledged_unverified", category: RecipientTransportCategory | "revocation_unverified") { super("share revocation outcome could not be verified"); this.name = "ShareRevocationError"; this.effect = effect; this.category = category === "revocation_unverified" ? category : safeTransportCategory(category); }
}
/** DELETE is never retried; an acknowledgement is followed by a complete upload-scope readback. */
export async function revokeShare(options: ShareOptions, managementId: string): Promise<void> {
  if (typeof managementId !== "string" || !ID.test(managementId)) throw new RecipientTransportError("invalid_request");
  const pinned = snapshot({ ...options, scope: "upload" });
  if (options.scope !== "upload") throw new RecipientTransportError("auth_required");
  try {
    const bytes = await scopedRequest(pinned, pinned.server, `/v1/shares/${managementId}`, "DELETE");
    let reply: unknown; try { reply = decodeStrictJson(bytes, 2_000_000); } catch { throw new ShareRevocationError("unknown", "invalid_response"); } finally { bytes.fill(0); }
    const body = object(reply); if (body.ok !== true) throw new ShareRevocationError("unknown", "invalid_response");
  } catch (error) { if (error instanceof ShareRevocationError) throw error; if (error instanceof RecipientTransportError) throw new ShareRevocationError("unknown", error.category); throw new ShareRevocationError("unknown", "request_failed"); }
  try {
    const remaining = await listShares(pinned);
    checkActive(pinned, pinned.signal ?? new AbortController().signal);
    if (!remaining.shares.some(row => row.id === managementId)) return;
    const row = remaining.shares.find(item => item.id === managementId)!;
    if (row.status === "revoked" || row.status === "expired") return;
    throw new ShareRevocationError("acknowledged_unverified", "revocation_unverified");
  } catch (error) { if (error instanceof ShareRevocationError) throw error; if (error instanceof RecipientTransportError) throw new ShareRevocationError("acknowledged_unverified", error.category); throw new ShareRevocationError("acknowledged_unverified", "revocation_unverified"); }
}

import { createHash } from "node:crypto";
import { verifyEnvelope, type Asset, type Context, type Signed } from "../recipient/crypto.ts";
import { validateRecipientEnvelopeBytesWithVerifier } from "../recipient-envelope.ts";
import { RecipientTransportError } from "../recipient/transport.ts";
import { checkActive, fail, json, normalizeServer, readPreparedAsset, request, type Authorization } from "./request.ts";

export type EncryptedProfile = "root" | "evidence-first-v1";
export type EncryptedUploadAuthorization = Authorization & { accountId: string; senderGrantId: string; recipientGrantIds: readonly string[] };
export type PreparedEncryptedGroup = { context: Context; envelope: Asset; chunks: readonly Asset[] };
export type PreparedEncryptedCapture = {
  server: string; captureId: string; profile: EncryptedProfile; context: Context; recipientGrantIds: readonly string[];
  envelope: Asset; chunks: readonly Asset[]; readAsset(path: string, signal: AbortSignal): Promise<Uint8Array>;
  /** Grouped profile carries both immutable signed group snapshots. */
  groups?: readonly PreparedEncryptedGroup[];
  trustedSenderSigningKey: Uint8Array;
  authorize(server: string, scope: "upload", signal: AbortSignal): Promise<EncryptedUploadAuthorization>;
  active(): boolean; signal?: AbortSignal;
};
const LIMIT = 128 * 1024, CHUNK = 32 * 1024 * 1024, TOTAL = 500_000_000, ID = /^[a-f0-9]{32}$/;
const SAFE = new Set(["invalid_request", "invalid_response", "invalid_server", "stale_snapshot", "auth_required", "unauthorized", "forbidden", "redirect_refused", "cancelled", "timeout", "response_too_large", "request_failed"]);
function manifest(files: readonly Asset[]): Asset[] {
  if (!Array.isArray(files) || files.length < 2 || files.length > 17) fail("invalid_request");
  const out: Asset[] = [], names = new Set<string>(); let sum = 0;
  for (const f of files) { if (!f || typeof f.path !== "string" || f.path.length > 128 || !Number.isSafeInteger(f.size) || f.size <= 0 || typeof f.sha256 !== "string" || !/^[a-f0-9]{64}$/.test(f.sha256) || names.has(f.path)) fail("invalid_request"); names.add(f.path); sum += f.size; out.push({ path: f.path, size: f.size, sha256: f.sha256 }); }
  if (sum > TOTAL) fail("invalid_request"); return out;
}
function exactSet(a: readonly string[], b: readonly string[]): boolean { return a.length === b.length && new Set(a).size === a.length && new Set(b).size === b.length && a.every(x => b.includes(x)); }
function sameManifest(a: readonly Asset[], b: unknown): boolean {
  if (!Array.isArray(b) || b.length !== a.length) return false; const map = new Map(a.map(f => [f.path, f]));
  return b.every(x => x && typeof x === "object" && !Array.isArray(x) && Object.keys(x).sort().join(",") === "path,sha256,size" && typeof x.path === "string" && Number.isSafeInteger(x.size) && typeof x.sha256 === "string" && map.get(x.path)?.size === x.size && map.get(x.path)?.sha256 === x.sha256);
}
function checkAuth(a: EncryptedUploadAuthorization, server: string, input: Pick<PreparedEncryptedCapture, "context" | "recipientGrantIds">): void {
  if (!a || a.approvedServer !== server || a.scope !== "upload" || a.accountId !== input.context.accountId || a.senderGrantId !== input.context.senderGrantId || !Array.isArray(a.recipientGrantIds) || !exactSet(a.recipientGrantIds, input.recipientGrantIds)) fail("auth_required");
}
export async function uploadPreparedEncryptedCapture(input: PreparedEncryptedCapture): Promise<void> {
  try {
    const readAsset = input.readAsset, authorizeCaller = input.authorize, active = input.active, signal = input.signal;
    const captureId = input.captureId, serverInput = input.server, profile = input.profile;
    const context: Context = { origin: input.context.origin, accountId: input.context.accountId, captureId: input.context.captureId, senderGrantId: input.context.senderGrantId };
    const recipients = [...input.recipientGrantIds], key = new Uint8Array(input.trustedSenderSigningKey);
    const options = { server: serverInput, active, signal, authorize: authorizeCaller };
    const server = normalizeServer(serverInput);
    if (!ID.test(captureId) || context.captureId !== captureId || typeof readAsset !== "function" || typeof authorizeCaller !== "function" || typeof active !== "function" || key.length !== 32 || !recipients.length || recipients.some(x => typeof x !== "string" || !x.length) || new Set(recipients).size !== recipients.length || (profile !== "root" && profile !== "evidence-first-v1")) fail("invalid_request");
    const grouped = profile === "evidence-first-v1";
    if (context.origin !== server || input.context.group !== undefined) fail("invalid_request");
    const groups: PreparedEncryptedGroup[] = grouped ? [...(input.groups ?? [])].map(g => ({ context: { ...g.context }, envelope: { ...g.envelope }, chunks: g.chunks.map(c => ({ ...c })) }))
      : [{ context, envelope: { ...input.envelope }, chunks: input.chunks.map(c => ({ ...c })) }];
    if (grouped ? groups.length !== 2 || !["evidence", "media"].every(g => groups.filter(x => x.context.group === g).length === 1) : groups.length !== 1) fail("invalid_request");
    const files: Asset[] = [], envelopes: Array<{ asset: Asset; expected: Context; chunks: Asset[]; prefix: string }> = [];
    for (const g of groups) {
      const group = g.context.group, prefix = grouped ? `${group}/` : "";
      if (g.context.origin !== server || g.context.accountId !== context.accountId || g.context.captureId !== context.captureId || g.context.senderGrantId !== context.senderGrantId || (grouped ? !["evidence", "media"].includes(group ?? "") : group !== undefined)) fail("invalid_request");
      const envelopeAsset = g.envelope, chunkFiles = [...g.chunks];
      if (envelopeAsset.path !== `${prefix}envelope.json` || envelopeAsset.size > LIMIT || chunkFiles.length < 1 || chunkFiles.length > 16 || chunkFiles.some((f, i) => f.path !== `${prefix}sealed/part-${String(i).padStart(6, "0")}.age` || f.size > CHUNK || (i < chunkFiles.length - 1 && f.size !== CHUNK))) fail("invalid_request");
      files.push(envelopeAsset, ...chunkFiles); envelopes.push({ asset: envelopeAsset, expected: g.context, chunks: chunkFiles, prefix });
    }
    const immutableFiles = manifest(files);
    if (new TextEncoder().encode(JSON.stringify({ format: "apparent-encrypted-v1", ...(grouped ? { delivery: "evidence-first-v1" } : {}), files: immutableFiles })).byteLength > LIMIT) fail("invalid_request");
    // Verify signed plaintext chunk manifest and context before the first reservation.
    checkActive(options, signal ?? new AbortController().signal);
    for (const group of envelopes) {
      const source = await readPreparedAsset({ ...options, readAsset }, group.asset.path);
      if (!(source instanceof Uint8Array) || source.byteLength !== group.asset.size) fail("stale_snapshot");
      const rawEnvelope = new Uint8Array(source);
      try {
      if (createHash("sha256").update(rawEnvelope).digest("hex") !== group.asset.sha256) fail("stale_snapshot");
      const verified = await validateRecipientEnvelopeBytesWithVerifier(rawEnvelope, group.expected, key, verifyEnvelope).catch(() => fail("invalid_request"));
      const expectedChunkPaths = group.chunks.map(f => ({ ...f, path: f.path.slice(group.prefix.length) }));
      if (JSON.stringify(verified.signed.chunks) !== JSON.stringify(expectedChunkPaths)) fail("invalid_request");
      } finally { rawEnvelope.fill(0); }
    }
    const auth = async (s: string, scope: "upload", requestSignal: AbortSignal) => { const value = await authorizeCaller(s, scope, requestSignal); const facts = { approvedServer: value?.approvedServer, scope: value?.scope, token: value?.token, accountId: value?.accountId, senderGrantId: value?.senderGrantId, recipientGrantIds: Array.isArray(value?.recipientGrantIds) ? [...value.recipientGrantIds] : [] } as EncryptedUploadAuthorization; checkAuth(facts, server, { context, recipientGrantIds: recipients }); return facts; };
    const transportOptions = { ...options, authorize: auth };
    const base = `/v1/captures/${captureId}`, reservationBody = new TextEncoder().encode(JSON.stringify({ format: "apparent-encrypted-v1", ...(grouped ? { delivery: "evidence-first-v1" } : {}), files: immutableFiles }));
    const reservation = json(await request(transportOptions, server, base, "POST", reservationBody, true));
    if (reservation.id !== captureId || (reservation.status !== "pending" && reservation.status !== "ready") || reservation.format !== "apparent-encrypted-v1" || (grouped ? reservation.delivery !== "evidence-first-v1" : reservation.delivery !== undefined)) fail("invalid_response");
    if (reservation.status === "pending") {
      for (const file of immutableFiles) { checkActive(options, signal ?? new AbortController().signal); const source = await readPreparedAsset({ ...options, readAsset }, file.path); if (!(source instanceof Uint8Array) || source.byteLength !== file.size) fail("stale_snapshot"); const owned = new Uint8Array(source); try { if (createHash("sha256").update(owned).digest("hex") !== file.sha256) fail("stale_snapshot"); await request(transportOptions, server, `${base}/files/${file.path}`, "PUT", owned, false); } finally { owned.fill(0); } }
      const complete = json(await request(transportOptions, server, `${base}/complete`, "POST", undefined, true)); if (complete.id !== captureId || complete.status !== "ready" || complete.format !== "apparent-encrypted-v1") fail("invalid_response");
    }
    const ready = json(await request(transportOptions, server, base, "GET", undefined, true));
    if (ready.id !== captureId || ready.status !== "ready" || ready.format !== "apparent-encrypted-v1" || (grouped ? ready.delivery !== "evidence-first-v1" : ready.delivery !== undefined) || !sameManifest(immutableFiles, ready.files)) fail("invalid_response");
  } catch (cause) { if (cause instanceof RecipientTransportError && SAFE.has(cause.category)) fail(cause.category); fail("request_failed"); }
}

/** Bounded reader for the existing Rust StoredSpool v1 JSON receipt. */
import type { Asset, Context } from "../recipient/crypto.ts";
import { decodeStrictJson } from "../recipient/json.ts";
import { exact, record, sameContext } from "../recipient/manifest.ts";

export type StoredEncryptedSpool = Readonly<{
  metadata: Readonly<{ context: Readonly<Context>; recipientGrants: readonly string[] }>;
  files: readonly Readonly<Asset>[];
}>;
export type StoredSpoolProfile = "root" | "evidence-first-v1";
export class StoredSpoolDecodeError extends Error {
  constructor() { super("invalid_encrypted_spool"); this.name = "StoredSpoolDecodeError"; }
}
function invalid(): never { throw new StoredSpoolDecodeError(); }
function immutableContext(value: Record<string, unknown>): Context {
  const context: Context = {
    origin: value.origin as string, accountId: value.accountId as string,
    captureId: value.captureId as string, senderGrantId: value.senderGrantId as string,
    ...(value.group == null ? {} : { group: value.group as "evidence" | "media" }),
  };
  return Object.freeze(context);
}
function validateFiles(value: unknown, profile: StoredSpoolProfile): Asset[] {
  const grouped = profile === "evidence-first-v1";
  const maxCount = grouped ? 34 : 17, chunk = 32 * 1024 * 1024;
  if (!Array.isArray(value) || value.length < 2 || value.length > maxCount) return invalid();
  const names = new Set<string>(); let total = 0;
  const files: Asset[] = [];
  for (const raw of value) {
    // Rust Asset ignores unknown fields; the read-only DTO keeps known facts.
    if (!record(raw) || typeof raw.path !== "string"
      || raw.path.length > 128 || !Number.isSafeInteger(raw.size) || (raw.size as number) <= 0
      || typeof raw.sha256 !== "string" || !/^[a-f0-9]{64}$/.test(raw.sha256) || names.has(raw.path)) return invalid();
    const path = raw.path, size = raw.size as number;
    const prefix = grouped ? (path.startsWith("evidence/") ? "evidence/" : path.startsWith("media/") ? "media/" : "") : "";
    if (grouped && !prefix) return invalid();
    const local = grouped ? path.slice(prefix.length) : path;
    const index = local.match(/^sealed\/part-(\d{6})\.age$/);
    const isEnvelope = local === "envelope.json";
    if (isEnvelope ? size > 128 * 1024 : !index || Number(index[1]) >= 16 || size > chunk) return invalid();
    names.add(path); total += size;
    if (total > 500_000_000) return invalid();
    files.push({ path, size, sha256: raw.sha256 });
  }
  const groups = grouped ? ["evidence/", "media/"] : [""];
  for (const prefix of groups) {
    const selected = files.filter(file => file.path.startsWith(prefix)).map(file => ({ ...file, path: file.path.slice(prefix.length) }));
    if (selected.length < 2 || selected.filter(file => file.path === "envelope.json").length !== 1) return invalid();
    const chunks = selected.filter(file => file.path !== "envelope.json").sort((a,b) => a.path.localeCompare(b.path));
    if (chunks.length < 1 || chunks.length > 16 || chunks.some((file, i) => file.path !== `sealed/part-${String(i).padStart(6,"0")}.age` || (i < chunks.length-1 && file.size !== chunk))) return invalid();
  }
  return files;
}

/** Decode the JSON shape Rust serde writes. The pinned context is supplied by the caller. */
export function decodeStoredEncryptedSpool(bytes: Uint8Array, expected: { context: Context; profile: StoredSpoolProfile }): StoredEncryptedSpool {
  try {
    if (expected.profile !== "root" && expected.profile !== "evidence-first-v1" || expected.context.group !== undefined) return invalid();
    const value = decodeStrictJson(bytes, 2_000_000);
    if (!record(value) || !exact(value, ["metadata", "files"]) || !record(value.metadata)
      || !exact(value.metadata, ["context", "recipientGrants"]) || !record(value.metadata.context)) return invalid();
    const rawContext = value.metadata.context;
    if (!exact(rawContext, ["origin", "accountId", "captureId", "senderGrantId", ...(Object.hasOwn(rawContext, "group") ? ["group"] : [])])
      || ![rawContext.origin, rawContext.accountId, rawContext.captureId, rawContext.senderGrantId].every(x => typeof x === "string")
      || (rawContext.group !== undefined && rawContext.group !== null && rawContext.group !== "evidence" && rawContext.group !== "media")
      || !sameContext(rawContext, expected.context) || !Array.isArray(value.metadata.recipientGrants)
      || value.metadata.recipientGrants.length > 100_000) return invalid();
    const grants = value.metadata.recipientGrants;
    let grantBytes = 2;
    for (const grant of grants) {
      if (typeof grant !== "string" || !grant.length || new TextEncoder().encode(grant).length > 256) return invalid();
      grantBytes += new TextEncoder().encode(JSON.stringify(grant)).length + 1;
      if (grantBytes > 2_000_000) return invalid();
    }
    if (new Set(grants).size !== grants.length) return invalid();
    const files = validateFiles(value.files, expected.profile);
    const context = immutableContext(rawContext);
    const metadata = Object.freeze({ context, recipientGrants: Object.freeze([...grants]) });
    return Object.freeze({ metadata, files: Object.freeze(files.map(file => Object.freeze({ ...file }))) });
  } catch { return invalid(); }
}

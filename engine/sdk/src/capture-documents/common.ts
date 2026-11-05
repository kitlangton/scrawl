import { createHash } from "node:crypto";
import { isAbsolute, relative, resolve, sep } from "node:path";
import type { FileIdentity, MaterialsKey, SealedFile } from "./types.ts";

export const MAX_DOCUMENT_BYTES = 8 * 1024 * 1024;
const idPattern = /^[0-9a-f]{32}$/;
const hashPattern = /^[0-9a-f]{64}$/;

export function fail(message: string): never { throw new Error(`capture document: ${message}`); }
export function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${label} must be an object`);
  return value as Record<string, unknown>;
}
export function exact(value: Record<string, unknown>, fields: readonly string[], label: string): void {
  if (Object.keys(value).some((key) => !fields.includes(key)) || fields.some((key) => !(key in value))) fail(`${label} has invalid fields`);
}
export function optionalExact(value: Record<string, unknown>, fields: readonly string[], required: readonly string[], label: string): void {
  if (Object.keys(value).some((key) => !fields.includes(key)) || required.some((key) => !(key in value))) fail(`${label} has invalid fields`);
}
export function string(value: unknown, label: string): asserts value is string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0")) fail(`${label} must be a nonempty string`);
}
export function id(value: unknown, label: string): asserts value is string {
  if (typeof value !== "string" || !idPattern.test(value)) fail(`${label} must be a 32-character lowercase hex id`);
}
export function sha(value: unknown, label: string): asserts value is string {
  if (typeof value !== "string" || !hashPattern.test(value)) fail(`${label} must be a 64-character lowercase SHA256`);
}
export function safeInt(value: unknown, label: string, min = 0): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < min) fail(`${label} must be a safe integer >= ${min}`);
}
export function absolutePath(value: unknown, label: string): asserts value is string {
  string(value, label);
  if (value.length > 4096 || !isAbsolute(value) || resolve(value) !== value) fail(`${label} must be normalized and absolute`);
}
export function relativePath(value: unknown, label: string): asserts value is string {
  string(value, label);
  if (value.length > 4096 || value.startsWith("/") || value.includes("\\") || value.split("/").some((part) => !part || part === "." || part === "..")) fail(`${label} must be a normalized contained relative path`);
}
export function contained(root: string, path: string, label: string): void {
  const out = relative(root, resolve(root, path));
  if (!out || out === ".." || out.startsWith(`..${sep}`) || isAbsolute(out)) fail(`${label} escapes its root`);
}
export function parseDetached<T>(bytes: Uint8Array, expectedHash: string, label: string): T {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength === 0 || bytes.byteLength > MAX_DOCUMENT_BYTES) fail(`${label} bytes exceed bounds`);
  sha(expectedHash, `${label} SHA256`);
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== expectedHash) fail(`${label} exact-byte SHA256 mismatch`);
  let decoded: string;
  try { decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes); }
  catch { return fail(`${label} is not valid UTF-8`); }
  try { return JSON.parse(decoded) as T; }
  catch { return fail(`${label} is not valid JSON`); }
}
export function validateIdentity(value: unknown, label: string): asserts value is FileIdentity {
  const x = record(value, label); exact(x, ["device", "inode", "size", "modifiedNs"], label);
  string(x.device, `${label}.device`); string(x.inode, `${label}.inode`); safeInt(x.size, `${label}.size`); string(x.modifiedNs, `${label}.modifiedNs`);
}
export function validateSealedFile(value: unknown, label: string): asserts value is SealedFile {
  const x = record(value, label); exact(x, ["relativePath", "identity", "sha256"], label);
  relativePath(x.relativePath, `${label}.relativePath`); validateIdentity(x.identity, `${label}.identity`); sha(x.sha256, `${label}.sha256`);
}
export function validateMaterialsKey(value: unknown, label = "materials"): asserts value is MaterialsKey {
  const x = record(value, label); exact(x, ["captureId", "materialsId", "revision"], label);
  id(x.captureId, `${label}.captureId`); id(x.materialsId, `${label}.materialsId`); sha(x.revision, `${label}.revision`);
}
export function sameMaterials(a: MaterialsKey, b: MaterialsKey, label: string): void {
  if (a.captureId !== b.captureId || a.materialsId !== b.materialsId || a.revision !== b.revision) fail(`${label} materials identity mismatch`);
}
export function assertProcessingAllowed(materials: { payload: { safety: { safeForProcessing: boolean } } }): void {
  if (!materials.payload.safety.safeForProcessing) fail("capture materials are not safe for processing or publication");
}
export function validateMaterialEnvelopeIdentity(payload: { identity: { captureId: string; materialsId: string }; root: string; leaseId: string }, envelope: { key: MaterialsKey; root: string; leaseId: string; payloadFile: SealedFile }): void {
  if (envelope.key.captureId !== payload.identity.captureId || envelope.key.materialsId !== payload.identity.materialsId) fail("envelope/payload identity mismatch");
  if (envelope.root !== payload.root) fail("envelope/payload root mismatch");
  if (envelope.leaseId !== payload.leaseId) fail("envelope/payload lease mismatch");
  if (envelope.key.revision !== envelope.payloadFile.sha256) fail("materials revision does not bind exact payload bytes");
}

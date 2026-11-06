import { createHash } from "node:crypto";
import { constants } from "node:fs";
import type { BigIntStats } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, join, parse, resolve } from "node:path";
import { validateFactFile, validateMaterialsDocument, type MaterialsDocument, type MaterialsKey, type MaterialsManifest } from "@apparent/engine-sdk/capture";

export interface CaptureSealedFile { relativePath: string; identity: { device: string; inode: string; size: number; modifiedNs: string }; sha256: string }

export type CaptureMaterialFactKind = "context" | "interactions" | "candidates" | "transcript" | "acceptedStrokes";
export interface CaptureMaterialsExpected { key: MaterialsKey; leaseId: string; bundleId: string }
export interface CaptureMaterialsLimits { maxFileBytes?: number; maxAggregateBytes?: number }
export interface CaptureFactInput { file: CaptureSealedFile; bytes: Uint8Array }
export interface CaptureMaterialsInput {
  manifest: MaterialsManifest;
  expected: CaptureMaterialsExpected;
  /** Independently trusted immutable retained-materials root. */
  trustedRoot: string;
  limits?: CaptureMaterialsLimits;
}
export interface CaptureMaterialsRead {
  materials: MaterialsDocument;
  /** Original detached bytes: re-encoding the parsed payload would change its sealed revision. */
  payloadBytes: Uint8Array;
  /** Only collected states have entries. Unavailable observations remain in materials.payload.collection. */
  facts: Partial<Record<CaptureMaterialFactKind, CaptureFactInput>>;
  /** Assembly-compatible subset, preserving the exact public SealedFile envelopes. */
  assemblyFacts: Partial<Record<"context" | "interactions" | "transcript" | "acceptedStrokes", CaptureFactInput>>;
}

const DEFAULT_FILE_LIMIT = 8 * 1024 * 1024;
const DEFAULT_TOTAL_LIMIT = 20 * 1024 * 1024;
const HARD_FILE_LIMIT = 64 * 1024 * 1024;
const HARD_TOTAL_LIMIT = 128 * 1024 * 1024;
const kinds: readonly CaptureMaterialFactKind[] = ["context", "interactions", "candidates", "transcript", "acceptedStrokes"];
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const fail = (message: string): never => { throw new TypeError(`capture materials read rejected: ${message}`); };
function safeRelative(relative: string): string {
  if (!relative || relative.includes("\0") || isAbsolute(relative) || /^[A-Za-z]:/u.test(relative) || relative.split(/[\\/]/u).some(part => !part || part === "." || part === "..")) fail("invalid or escaping sealed path");
  return relative.replaceAll("\\", "/");
}
function statIdentity(stat: BigIntStats): CaptureSealedFile["identity"] {
  return { device: String(stat.dev), inode: String(stat.ino), size: Number(stat.size), modifiedNs: String(stat.mtimeNs) };
}
async function checkRoot(root: string): Promise<void> {
  if (!isAbsolute(root) || resolve(root) !== root || root.includes("\0") || root === parse(root).root) fail("trusted root must be absolute and normalized");
  const volume = parse(root).root;
  let current = volume;
  for (const component of root.slice(volume.length).split(/[\\/]+/u).filter(Boolean)) {
    current = join(current, component);
    const stat = await lstat(current).catch(() => fail("trusted root or parent is unavailable"));
    if (!stat.isDirectory() || stat.isSymbolicLink()) fail("trusted root or parent is symlinked or not a directory");
  }
  if (await realpath(root).catch(() => "") !== root) fail("trusted root aliases another path");
}
async function readSealed(root: string, file: CaptureSealedFile, limits: { maxFileBytes: number }, remaining: number): Promise<Uint8Array> {
  const relative = safeRelative(file.relativePath);
  const path = join(root, ...relative.split("/"));
  const parentParts = relative.split("/").slice(0, -1);
  let parent = root;
  for (const part of parentParts) {
    parent = join(parent, part);
    const stat = await lstat(parent).catch(() => fail("sealed path parent is unavailable"));
    if (!stat.isDirectory() || stat.isSymbolicLink()) fail("sealed path parent is symlinked or not a directory");
  }
  const beforePath = await lstat(path, { bigint: true }).catch(() => fail("sealed file is unavailable"));
  if (!beforePath.isFile() || beforePath.isSymbolicLink() || beforePath.nlink !== 1n) fail("sealed file is not a regular private path");
  if (beforePath.size > BigInt(limits.maxFileBytes) || beforePath.size > BigInt(remaining)) fail("sealed file exceeds byte limit");
  if (constants.O_NOFOLLOW === undefined) fail("platform does not support no-follow file opens");
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW).catch(() => fail("sealed file could not be opened without following links"));
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile() || String(before.dev) !== file.identity.device || String(before.ino) !== file.identity.inode || Number(before.size) !== file.identity.size || String(before.mtimeNs) !== file.identity.modifiedNs || before.size > BigInt(limits.maxFileBytes) || before.size > BigInt(remaining)) fail("sealed file identity changed or exceeds byte limit");
    const chunks: Buffer[] = [];
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let total = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, Math.min(buffer.length, limits.maxFileBytes - total + 1), null);
      if (!bytesRead) break;
      total += bytesRead;
      if (total > limits.maxFileBytes || total > remaining) fail("sealed file exceeds byte limit");
      chunks.push(Buffer.from(buffer.subarray(0, bytesRead)));
    }
    const after = await handle.stat({ bigint: true });
    const afterPath = await lstat(path, { bigint: true }).catch(() => fail("sealed file was replaced while reading"));
    const identity = statIdentity(beforePath);
    if (!same(statIdentity(after), identity) || !same(statIdentity(afterPath), identity) || String(after.dev) !== file.identity.device || String(after.ino) !== file.identity.inode || total !== file.identity.size) fail("sealed file changed while reading");
    const bytes = Buffer.concat(chunks, total);
    if (createHash("sha256").update(bytes).digest("hex") !== file.sha256) fail("sealed file hash mismatch");
    return new Uint8Array(bytes);
  } finally { await handle.close(); }
}

/** Read detached materials and declared facts. This verifies local bytes, not native authority or media validity. */
export async function readCaptureMaterials(input: CaptureMaterialsInput): Promise<CaptureMaterialsRead> {
  const snapshot = structuredClone(input);
  const maxFileBytes = snapshot.limits?.maxFileBytes ?? DEFAULT_FILE_LIMIT;
  const maxAggregateBytes = snapshot.limits?.maxAggregateBytes ?? DEFAULT_TOTAL_LIMIT;
  if (!Number.isSafeInteger(maxFileBytes) || maxFileBytes < 1 || maxFileBytes > HARD_FILE_LIMIT || !Number.isSafeInteger(maxAggregateBytes) || maxAggregateBytes < 1 || maxAggregateBytes > HARD_TOTAL_LIMIT || maxFileBytes > maxAggregateBytes) fail("invalid byte limits");
  const { manifest, expected, trustedRoot } = snapshot;
  await checkRoot(trustedRoot);
  if (manifest.root !== trustedRoot || manifest.payloadFile.relativePath !== safeRelative(manifest.payloadFile.relativePath)) fail("manifest root/path alias mismatch");
  let aggregate = 0;
  const payloadBytes = await readSealed(trustedRoot, manifest.payloadFile, { maxFileBytes }, maxAggregateBytes);
  aggregate += payloadBytes.byteLength;
  const materials = validateMaterialsDocument(manifest, payloadBytes);
  const key = materials.manifest.key;
  if (key.captureId !== expected.key.captureId || key.materialsId !== expected.key.materialsId || key.revision !== expected.key.revision || materials.manifest.leaseId !== expected.leaseId || materials.payload.bundleId !== expected.bundleId || materials.payload.root !== trustedRoot || materials.payload.leaseId !== expected.leaseId || !materials.payload.safety.safeForProcessing) fail("materials identity, lease, root, bundle, or processing safety mismatch");
  const byPath = new Map<string, CaptureSealedFile>();
  for (const file of materials.payload.files) {
    const path = safeRelative(file.relativePath);
    if (path === manifest.payloadFile.relativePath || byPath.has(path)) fail("duplicate or conflicting sealed material path");
    byPath.set(path, file as CaptureSealedFile);
  }
  const facts: CaptureMaterialsRead["facts"] = {};
  for (const kind of kinds) {
    const state = materials.payload.collection[kind];
    const ref = materials.payload.facts[kind];
    if (state === "unavailable") { if (ref) fail(`${kind} is unavailable but has a file reference`); continue; }
    if (!ref) throw new TypeError(`capture materials read rejected: ${kind} is collected but has no file reference`);
    const relative = safeRelative(ref.file);
    if (relative !== ref.file) fail(`${kind} fact reference is not canonical`);
    const file = byPath.get(relative);
    if (!file) throw new TypeError(`capture materials read rejected: ${kind} fact has no sealed file identity`);
    if (file.relativePath !== ref.file || file.sha256.length !== 64) fail(`${kind} fact reference is malformed`);
    const bytes = await readSealed(trustedRoot, file, { maxFileBytes }, maxAggregateBytes - aggregate);
    aggregate += bytes.byteLength;
    validateFactFile(file, bytes, materials, kind);
    facts[kind] = { file: structuredClone(file), bytes };
  }
  const assemblyFacts: CaptureMaterialsRead["assemblyFacts"] = {};
  for (const kind of ["context", "interactions", "transcript", "acceptedStrokes"] as const) if (facts[kind]) assemblyFacts[kind] = facts[kind];
  return { materials: structuredClone(materials), payloadBytes, facts, assemblyFacts };
}

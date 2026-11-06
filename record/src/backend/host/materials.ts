import { validateFactFile, validateMaterialsDocument, type MaterialsDocument, type MaterialsKey, type MaterialsManifest } from "@apparent/engine-sdk/capture";
import { assertSealedRoot, normalizeSealedRelative, readSealedFile } from "../sealed-files.js";

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
const fail = (message: string): never => { throw new TypeError(`capture materials read rejected: ${message}`); };

/** Read detached materials and declared facts. This verifies local bytes, not native authority or media validity. */
export async function readCaptureMaterials(input: CaptureMaterialsInput): Promise<CaptureMaterialsRead> {
  const snapshot = structuredClone(input);
  const maxFileBytes = snapshot.limits?.maxFileBytes ?? DEFAULT_FILE_LIMIT;
  const maxAggregateBytes = snapshot.limits?.maxAggregateBytes ?? DEFAULT_TOTAL_LIMIT;
  if (!Number.isSafeInteger(maxFileBytes) || maxFileBytes < 1 || maxFileBytes > HARD_FILE_LIMIT || !Number.isSafeInteger(maxAggregateBytes) || maxAggregateBytes < 1 || maxAggregateBytes > HARD_TOTAL_LIMIT || maxFileBytes > maxAggregateBytes) fail("invalid byte limits");
  const { manifest, expected, trustedRoot } = snapshot;
  await assertSealedRoot(trustedRoot, "capture materials");
  if (manifest.root !== trustedRoot || manifest.payloadFile.relativePath !== normalizeSealedRelative(manifest.payloadFile.relativePath, "capture materials")) fail("manifest root/path alias mismatch");
  let aggregate = 0;
  const payloadBytes = await readSealedFile(trustedRoot, manifest.payloadFile, maxFileBytes, maxAggregateBytes, "capture materials");
  aggregate += payloadBytes.byteLength;
  const materials = validateMaterialsDocument(manifest, payloadBytes);
  const key = materials.manifest.key;
  if (key.captureId !== expected.key.captureId || key.materialsId !== expected.key.materialsId || key.revision !== expected.key.revision || materials.manifest.leaseId !== expected.leaseId || materials.payload.bundleId !== expected.bundleId || materials.payload.root !== trustedRoot || materials.payload.leaseId !== expected.leaseId || !materials.payload.safety.safeForProcessing) fail("materials identity, lease, root, bundle, or processing safety mismatch");
  const byPath = new Map<string, CaptureSealedFile>();
  for (const file of materials.payload.files) {
    const path = normalizeSealedRelative(file.relativePath, "capture materials");
    if (path === manifest.payloadFile.relativePath || byPath.has(path)) fail("duplicate or conflicting sealed material path");
    byPath.set(path, file as CaptureSealedFile);
  }
  const facts: CaptureMaterialsRead["facts"] = {};
  for (const kind of kinds) {
    const state = materials.payload.collection[kind];
    const ref = materials.payload.facts[kind];
    if (state === "unavailable") { if (ref) fail(`${kind} is unavailable but has a file reference`); continue; }
    if (!ref) throw new TypeError(`capture materials read rejected: ${kind} is collected but has no file reference`);
    const relative = normalizeSealedRelative(ref.file, "capture materials");
    if (relative !== ref.file) fail(`${kind} fact reference is not canonical`);
    const file = byPath.get(relative);
    if (!file) throw new TypeError(`capture materials read rejected: ${kind} fact has no sealed file identity`);
    if (file.relativePath !== ref.file || file.sha256.length !== 64) fail(`${kind} fact reference is malformed`);
    const bytes = await readSealedFile(trustedRoot, file, maxFileBytes, maxAggregateBytes - aggregate, "capture materials");
    aggregate += bytes.byteLength;
    validateFactFile(file, bytes, materials, kind);
    facts[kind] = { file: structuredClone(file), bytes };
  }
  const assemblyFacts: CaptureMaterialsRead["assemblyFacts"] = {};
  for (const kind of ["context", "interactions", "transcript", "acceptedStrokes"] as const) if (facts[kind]) assemblyFacts[kind] = facts[kind];
  return { materials: structuredClone(materials), payloadBytes, facts, assemblyFacts };
}

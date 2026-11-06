import { isAbsolute } from "node:path";
import {
  selectDerivative2Artifact,
  validateDerivative2Document,
  type Derivative2DocumentInput,
  type Derivative2Selector,
} from "@apparent/engine-sdk/media2";
import type { MaterialsDocument } from "@apparent/engine-sdk/capture";
import { assertSealedRoot, readSealedFile } from "../sealed-files.js";
import type { SealedArtifact } from "./bundle/index.js";
import type { Derivative2ArtifactBinding } from "../publication/staging.js";

const HARD_FILE_LIMIT = 64 * 1024 * 1024;
const HARD_TOTAL_LIMIT = 128 * 1024 * 1024;
const fail = (message: string): never => { throw new TypeError(`retained derivative artifacts read rejected: ${message}`); };

export interface RetainedArtifactMapping {
  /** Explicit final bundle path, independent of the native receipt's relative path. */
  bundlePath: string;
  selector: Derivative2Selector;
}

export interface ReadRetainedDerivativeArtifactsInput {
  document: Derivative2DocumentInput;
  /** Original SDK-validated material document used to prepare this request. */
  materials: MaterialsDocument;
  /** Host-trusted immutable directory supplied separately from the receipt. */
  trustedRoot: string;
  /** Caller-selected native outputs and their final bundle references. */
  mappings: readonly RetainedArtifactMapping[];
  limits: { maxFileBytes: number; maxAggregateBytes: number };
}

export interface RetainedDerivativeArtifacts {
  /** BundleAssemblyInput-compatible bytes and original native seal metadata. */
  artifacts: SealedArtifact[];
  /** Exact path bindings consumed by prepareRecordPublication. */
  derivative2Artifacts: Derivative2ArtifactBinding;
}

function validateLimits(limits: ReadRetainedDerivativeArtifactsInput["limits"]): void {
  if (!Number.isSafeInteger(limits?.maxFileBytes) || limits.maxFileBytes < 1 || limits.maxFileBytes > HARD_FILE_LIMIT ||
      !Number.isSafeInteger(limits.maxAggregateBytes) || limits.maxAggregateBytes < 1 || limits.maxAggregateBytes > HARD_TOTAL_LIMIT ||
      limits.maxFileBytes > limits.maxAggregateBytes) fail("invalid finite byte limits");
}

function validateBundlePath(value: string): string {
  if (!value || value.includes("\0") || value.includes("\\") || isAbsolute(value) || /^[A-Za-z]:/u.test(value) ||
      value.split("/").some(part => !part || part === "." || part === "..")) fail("invalid, escaping, or non-canonical final bundle path");
  return value;
}

function selectorKey(selector: Derivative2Selector): string {
  return selector.kind === "frame" ? `frame:${selector.requestIndex}` : "contact-sheet";
}

/** Read explicitly selected original derivative seals for canonical assembly and staging. */
export async function readRetainedDerivativeArtifacts(input: ReadRetainedDerivativeArtifactsInput): Promise<RetainedDerivativeArtifacts> {
  validateLimits(input?.limits);
  if (!(input?.document?.bytes instanceof Uint8Array) || input.document.bytes.byteLength > input.limits.maxFileBytes) fail("detached derivative receipt exceeds per-file byte limit");
  const snapshot = structuredClone(input);
  if (!Array.isArray(snapshot.mappings) || snapshot.mappings.length === 0) fail("at least one explicit output mapping is required");
  if (snapshot.trustedRoot !== snapshot.document.request.outputDirectory) fail("trusted root differs from original request output root");

  // Validate the original detached receipt against the caller's original materials/request
  // before any filesystem read. This checks the request, layout, lease, job, and receipt.
  const envelope = validateDerivative2Document(snapshot.document, snapshot.materials);
  if (snapshot.trustedRoot !== envelope.root) fail("trusted root differs from validated receipt root");

  const finalPaths = new Set<string>();
  const selectors = new Set<string>();
  const selected: Array<{ bundlePath: string; selector: Derivative2Selector; artifact: ReturnType<typeof selectDerivative2Artifact> }> = [];
  for (const mapping of snapshot.mappings) {
    const bundlePath = validateBundlePath(mapping.bundlePath);
    if (finalPaths.has(bundlePath)) fail(`duplicate final bundle path ${bundlePath}`);
    finalPaths.add(bundlePath);
    const artifact = selectDerivative2Artifact(envelope, mapping.selector);
    const key = selectorKey(mapping.selector);
    if (selectors.has(key)) fail("one derivative selector cannot be assigned to multiple final paths");
    selectors.add(key);
    selected.push({ bundlePath, selector: structuredClone(mapping.selector), artifact });
  }

  await assertSealedRoot(snapshot.trustedRoot, "retained derivative artifacts");
  const artifacts: SealedArtifact[] = [];
  const bindings: Record<string, { receiptId: string; selector: Derivative2Selector }> = {};
  let aggregateBytes = 0;
  for (const item of selected) {
    const file = item.artifact.file;
    const remaining = snapshot.limits.maxAggregateBytes - aggregateBytes;
    const bytes = await readSealedFile(snapshot.trustedRoot, file, snapshot.limits.maxFileBytes, remaining, "retained derivative artifacts");
    if (bytes.byteLength !== item.artifact.byteLength || file.identity.size !== item.artifact.byteLength) fail("selected derivative length differs from its validated seal");
    aggregateBytes += bytes.byteLength;
    artifacts.push({
      materials: structuredClone(snapshot.materials.manifest.key),
      relativePath: item.bundlePath,
      bytes,
      sha256: file.sha256,
      byteLength: item.artifact.byteLength,
      identity: structuredClone(file.identity),
      role: item.selector.kind === "frame" ? "frame" : "contact-sheet",
    });
    Object.defineProperty(bindings, item.bundlePath, {
      value: { receiptId: envelope.receiptId, selector: structuredClone(item.selector) },
      enumerable: true,
      writable: false,
      configurable: false,
    });
  }
  return { artifacts, derivative2Artifacts: bindings };
}

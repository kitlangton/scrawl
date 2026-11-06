import { isDeepStrictEqual } from "node:util";
import { absolutePath, assertProcessingAllowed, contained, exact, fail, id, parseDetached, record, relativePath, safeInt, sha, sameMaterials, string, validateMaterialsKey, validateSealedFile } from "./common.ts";
import type { DerivativeDocumentInput, DerivativeEnvelope, MaterialsDocument, PublicationPlan, PublishEnvelope, SealedFile } from "./types.ts";
import { validateDerivativeEnvelope } from "./derivative.ts";
import type { Derivative2DocumentInput, Derivative2Envelope, Derivative2Selector } from "../media2/types.ts";
import { validateDerivative2Document, selectDerivative2Artifact } from "../media2/receipt.ts";

/** Draft inventory preflight. Native still authenticates provenance, validates
 * the complete ApparentBundle/media and enforces filesystem durability. */
export function validatePublicationPlan(envelope: unknown, planBytes: Uint8Array, materials: MaterialsDocument, derivatives: readonly DerivativeDocumentInput[] = [], derivatives2: readonly Derivative2DocumentInput[] = []): PublicationPlan {
  assertProcessingAllowed(materials);
  const e = record(envelope, "publish envelope"); exact(e, ["stagingRoot", "planFile", "planSha256"], "publish envelope");
  absolutePath(e.stagingRoot, "envelope stagingRoot"); validateSealedFile(e.planFile, "planFile"); sha(e.planSha256, "planSha256");
  const file = e.planFile as SealedFile; if (e.planSha256 !== file.sha256) fail("planSha256 does not match exact planFile hash");
  if (file.identity.size !== planBytes.byteLength) fail("publication plan byte length differs from sealed identity");
  if (derivatives.length + derivatives2.length > 32) fail("too many derivative receipts");
  const checked = new Map<string, DerivativeEnvelope>();
  for (const derivative of derivatives) {
    validateDerivativeEnvelope(derivative.envelope, derivative.bytes, materials, derivative.requestedBundleMs);
    if (checked.has(derivative.envelope.receiptId)) fail("duplicate derivative receipt identity");
    checked.set(derivative.envelope.receiptId, derivative.envelope);
  }
  const checked2 = new Map<string, Derivative2Envelope>();
  for (const derivative of derivatives2) {
    const receipt = validateDerivative2Document(derivative, materials);
    if (checked.has(receipt.receiptId) || checked2.has(receipt.receiptId)) fail("duplicate derivative receipt identity");
    checked2.set(receipt.receiptId, receipt);
  }
  const parsed = parseDetached<unknown>(planBytes, file.sha256, "publication plan"); validatePlan(parsed, materials, checked, checked2);
  if ((parsed as PublicationPlan).stagingRoot !== e.stagingRoot) fail("envelope/plan stagingRoot mismatch");
  return parsed as PublicationPlan;
}

function validatePlan(value: unknown, materials: MaterialsDocument, derivatives: ReadonlyMap<string, DerivativeEnvelope>, derivatives2: ReadonlyMap<string, Derivative2Envelope>): asserts value is PublicationPlan {
  const p = record(value, "publication plan"); exact(p, ["schema", "publicationId", "materials", "bundleId", "stagingRoot", "destination", "artifactVersion", "entries", "manifestPath", "summaryPath", "profile"], "publication plan");
  if (p.schema !== "apparent.bundle-publication/1" || p.artifactVersion !== 1 || p.profile !== "apparent-bundle/1" || p.manifestPath !== "apparent.json" || p.summaryPath !== "summary.md") fail("unsupported publication plan profile");
  id(p.publicationId, "publicationId"); validateMaterialsKey(p.materials); sameMaterials(p.materials as PublicationPlan["materials"], materials.manifest.key, "publication");
  string(p.bundleId, "bundleId"); if (p.bundleId !== materials.payload.bundleId) fail("publication bundle identity differs from materials"); absolutePath(p.stagingRoot, "plan stagingRoot");
  const destination = record(p.destination, "destination"); exact(destination, ["path", "mode"], "destination"); absolutePath(destination.path, "destination path"); if (destination.mode !== "absent") fail("publication destination must be absent-only");
  if (!Array.isArray(p.entries) || p.entries.length === 0 || p.entries.length > 4096) fail("publication inventory must be nonempty and bounded");
  const paths = new Set<string>(), roles = new Map<string, number>();
  p.entries.forEach((entry, i) => {
    const x = record(entry, `entries[${i}]`);
    exact(x, ["relativePath", "sha256", "byteLength", "role", "input"], `entries[${i}]`);
    relativePath(x.relativePath, "artifact path");
    if (paths.has(x.relativePath)) fail("duplicate publication inventory path");
    paths.add(x.relativePath);
    sha(x.sha256, "artifact sha256"); safeInt(x.byteLength, "artifact byteLength", 1);
    if (!["manifest", "summary", "companion", "frame", "contact-sheet", "video", "audio", "transcript", "context", "interactions", "marks"].includes(x.role as string)) fail("unsupported artifact role");
    roles.set(x.role as string, (roles.get(x.role as string) ?? 0) + 1);
    validateInput(x.input, materials, x, derivatives, derivatives2);
  });
  if (p.entries.filter((entry) => (entry as Record<string, unknown>).relativePath === "apparent.json" && (entry as Record<string, unknown>).role === "manifest").length !== 1) fail("profile requires apparent.json manifest");
  if (p.entries.filter((entry) => (entry as Record<string, unknown>).relativePath === "summary.md" && (entry as Record<string, unknown>).role === "summary").length !== 1) fail("profile requires summary.md");
  if (!roles.has("frame") || !roles.has("contact-sheet")) fail("profile requires nonempty frame and contact-sheet references");
  // This is an inventory/profile prerequisite, not the native full bundle/media gate.
}

function validateInput(value: unknown, materials: MaterialsDocument, entry: Record<string, unknown>, derivatives: ReadonlyMap<string, DerivativeEnvelope>, derivatives2: ReadonlyMap<string, Derivative2Envelope>): void {
  const input = record(value, "artifact input");
  if (input.kind === "material") {
    exact(input, ["kind", "relativePath"], "material artifact input"); relativePath(input.relativePath, "material input path");
    const source = materials.payload.files.find((file) => file.relativePath === input.relativePath);
    if (!source || source.sha256 !== entry.sha256 || source.identity.size !== entry.byteLength) fail("artifact inventory differs from sealed material");
  } else if (input.kind === "assembled") {
    exact(input, ["kind", "file"], "assembled artifact input"); validateSealedFile(input.file, "assembled file");
    const source = input.file as SealedFile;
    if (source.sha256 !== entry.sha256 || source.identity.size !== entry.byteLength) fail("artifact inventory differs from assembled input");
  } else if (input.kind === "derivative") {
    exact(input, ["kind", "receipt", "fileIndex"], "derivative artifact input"); safeInt(input.fileIndex, "derivative fileIndex", 1);
    const raw = record(input.receipt, "derivative receipt");
    id(raw.receiptId, "derivative receiptId");
    const receipt = derivatives.get(raw.receiptId);
    if (!receipt || !isDeepStrictEqual(raw, receipt)) fail("publication derivative requires matching detached receipt bytes");
    const frame = receipt.payload.frames[(input.fileIndex as number) - 1];
    if (!frame || frame.requestIndex !== input.fileIndex) fail("derivative fileIndex is outside receipt inventory");
    if (frame.file.sha256 !== entry.sha256 || frame.file.identity.size !== entry.byteLength) fail("publication derivative entry differs from referenced frame bytes");
  } else if (input.kind === "derivative2") {
    exact(input, ["kind", "receipt", "selector"], "derivative2 artifact input");
    const raw = record(input.receipt, "derivative2 receipt"); id(raw.receiptId, "derivative2 receiptId");
    const receipt = derivatives2.get(raw.receiptId);
    if (!receipt || !isDeepStrictEqual(raw, receipt)) fail("publication derivative2 requires matching detached receipt bytes and original request");
    const selector = input.selector as Derivative2Selector;
    const image = selectDerivative2Artifact(receipt, selector);
    if (entry.role !== (selector.kind === "frame" ? "frame" : "contact-sheet")) fail("derivative2 selector role mismatch");
    if (image.file.sha256 !== entry.sha256 || image.byteLength !== entry.byteLength) fail("publication derivative2 differs from selected sealed output");
  } else fail("unsupported artifact input provenance");
}

export function publicationPlanMatchesEnvelope(envelope: PublishEnvelope, planBytes: Uint8Array, materials: MaterialsDocument): PublicationPlan { return validatePublicationPlan(envelope, planBytes, materials); }

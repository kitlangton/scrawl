import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { contained, exact, fail, id, parseDetached, record, safeInt, sameMaterials, sha, validateMaterialsKey, validateSealedFile } from "../capture-documents/common.ts";
import { validateDerivativeFrameSlots } from "../capture-documents/derivative.ts";
import type { MaterialsDocument, SealedFile } from "../capture-documents/types.ts";
import { encodeContactSheetLayout, prepareMedia2Request } from "./request.ts";
import type { Derivative2DocumentInput, Derivative2Envelope, Derivative2Image, Derivative2Selector } from "./types.ts";

const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
/** Offline consistency checks. Only the native registry can authenticate origin. */
export function validateDerivative2Document(input: Derivative2DocumentInput, materials: MaterialsDocument): Derivative2Envelope {
  const prepared = prepareMedia2Request(input.request, materials);
  const e = record(input.envelope, "derivative2 envelope");
  exact(e, ["schema", "materials", "leaseId", "jobId", "receiptId", "root", "requestSha256", "receiptFile", "payload"], "derivative2 envelope");
  if (e.schema !== "apparent.media-derivative/2") fail("unsupported derivative2 envelope");
  validateMaterialsKey(e.materials); sameMaterials(e.materials as Derivative2Envelope["materials"], materials.manifest.key, "derivative2");
  id(e.receiptId, "derivative2 receiptId"); sha(e.requestSha256, "derivative2 request hash");
  if (e.leaseId !== prepared.request.leaseId || e.jobId !== prepared.request.jobId || e.root !== prepared.request.outputDirectory || e.requestSha256 !== prepared.sha256) fail("derivative2 original request binding mismatch");
  validateSealedFile(e.receiptFile, "derivative2 receiptFile");
  const seal = e.receiptFile as SealedFile;
  if (seal.identity.size !== input.bytes.byteLength) fail("derivative2 receipt length mismatch");
  const p = record(parseDetached<unknown>(input.bytes, seal.sha256, "derivative2 payload"), "derivative2 payload");
  exact(p, ["schema", "materials", "leaseId", "jobId", "receiptId", "root", "requestSha256", "frames", "contactSheet"], "derivative2 payload");
  if (p.schema !== "apparent.media-derivative-payload/2") fail("unsupported derivative2 payload");
  for (const key of ["materials", "leaseId", "jobId", "receiptId", "root", "requestSha256"] as const) if (!isDeepStrictEqual(p[key], e[key])) fail(`derivative2 envelope/payload ${key} mismatch`);
  if (!Array.isArray(p.frames)) fail("derivative2 frames missing");
  const paths = new Set<string>([seal.relativePath]);
  let pixels = 0;
  const image = (value: unknown, label: string): Derivative2Image => {
    const x = record(value, label);
    if (x.encoding !== "png") fail("derivative2 image encoding must be PNG");
    safeInt(x.byteLength, `${label} byteLength`, 1); validateSealedFile(x.file, `${label} file`);
    const file = x.file as SealedFile;
    if (x.byteLength !== file.identity.size) fail("derivative2 image sealed length mismatch");
    contained(e.root as string, file.relativePath, label);
    if (paths.has(file.relativePath)) fail("derivative2 image path aliases another output or receipt");
    paths.add(file.relativePath);
    safeInt(x.width, `${label} width`, 1); safeInt(x.height, `${label} height`, 1);
    pixels += x.width * x.height;
    return x as unknown as Derivative2Image;
  };
  // Reuse the existing clock/ordered-slot checks without changing derivative/1.
  const legacyFrames = p.frames.map((frame, index) => {
    const f = record(frame, `frames[${index}]`);
    exact(f, ["requestIndex", "requestedBundleMs", "effectiveVideoMs", "actualVideoMs", "clampedToStart", "clampedToEnd", "width", "height", "encoding", "byteLength", "file"], "derivative2 frame");
    image(f, "derivative2 frame"); safeInt(f.actualVideoMs, "actualVideoMs");
    if ((f.width as number) > prepared.request.limits.maxEdge || (f.height as number) > prepared.request.limits.maxEdge) fail("derivative2 frame exceeds request edge");
    const { encoding: _encoding, byteLength: _byteLength, ...legacy } = f;
    return legacy;
  });
  validateDerivativeFrameSlots(legacyFrames, materials, e.root as string, seal.relativePath, prepared.request.requestedBundleMs);
  if (prepared.request.contactSheet === null) {
    if (p.contactSheet !== null || input.layoutBytes !== null) fail("unrequested derivative2 sheet/layout");
  } else {
    const sheet = record(p.contactSheet, "derivative2 contact sheet");
    exact(sheet, ["encoding", "byteLength", "file", "width", "height", "layoutSha256", "layout"], "derivative2 contact sheet");
    image(sheet, "derivative2 contact sheet");
    const layout = encodeContactSheetLayout(prepared.request.contactSheet);
    if (!(input.layoutBytes instanceof Uint8Array) || !Buffer.from(input.layoutBytes).equals(Buffer.from(layout)) || sheet.layoutSha256 !== digest(layout) || !isDeepStrictEqual(sheet.layout, prepared.request.contactSheet)) fail("derivative2 layout bytes/hash/original layout mismatch");
    if (sheet.width !== prepared.request.contactSheet.width || sheet.height !== prepared.request.contactSheet.height) fail("derivative2 sheet dimensions mismatch");
  }
  if (pixels > prepared.request.limits.maxTotalPixels) fail("combined derivative2 frame and sheet pixel budget exceeded");
  if (!isDeepStrictEqual(e.payload, p)) fail("derivative2 detached payload differs from envelope");
  return structuredClone(e) as unknown as Derivative2Envelope;
}

/** Select an already validated receipt. The publisher independently authenticates
 * native job authority; these bytes and hashes alone never establish origin. */
export function selectDerivative2Artifact(receipt: Derivative2Envelope, selector: Derivative2Selector): Derivative2Image {
  const s = record(selector, "derivative2 selector");
  if (s.kind === "frame") {
    exact(s, ["kind", "requestIndex"], "derivative2 frame selector"); safeInt(s.requestIndex, "selector requestIndex", 1);
    const frame = receipt.payload.frames[s.requestIndex - 1];
    if (!frame || frame.requestIndex !== s.requestIndex) fail("derivative2 selector references missing frame slot");
    return frame;
  }
  exact(s, ["kind"], "derivative2 sheet selector");
  if (s.kind !== "contact-sheet" || !receipt.payload.contactSheet) fail("derivative2 selector references missing contact sheet");
  return receipt.payload.contactSheet;
}

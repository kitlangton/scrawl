import { absolutePath, contained, exact, fail, id, optionalExact, parseDetached, record, relativePath, safeInt, sha, string, validateMaterialEnvelopeIdentity, validateMaterialsKey, validateSealedFile } from "./common.ts";
import type { MaterialsDocument, MaterialsManifest, MaterialsPayload, SealedFile } from "./types.ts";
import { isDeepStrictEqual } from "node:util";

const factSchemas = { context: "apparent.capture-context/1", interactions: "apparent.capture-interactions/1", candidates: "apparent.capture-candidates/1", transcript: "apparent.capture-transcript/1", acceptedStrokes: "apparent.capture-strokes/1" } as const;

function validatePayload(value: unknown): asserts value is MaterialsPayload {
  const p = record(value, "materials payload");
  const fields = ["schema", "identity", "bundleId", "root", "leaseId", "clock", "confirmedSource", "invocation", "files", "video", "facts", "collection", "warnings", "safety"];
  optionalExact(p, [...fields, "audio"], fields, "materials payload");
  if (p.schema !== "apparent.capture-materials-payload/1") fail("unsupported materials payload schema");
  const identity = record(p.identity, "payload identity"); exact(identity, ["captureId", "materialsId"], "payload identity"); id(identity.captureId, "captureId"); id(identity.materialsId, "materialsId");
  string(p.bundleId, "bundleId"); absolutePath(p.root, "materials root"); id(p.leaseId, "leaseId");
  const clock = record(p.clock, "clock"); optionalExact(clock, ["epoch", "units", "durationMs", "videoOffsetMs", "audioOffsetMs"], ["epoch", "units", "durationMs", "videoOffsetMs"], "clock");
  if (clock.epoch !== "capture-admission-monotonic" || clock.units !== "integer-milliseconds") fail("unsupported capture clock");
  safeInt(clock.durationMs, "durationMs"); safeInt(clock.videoOffsetMs, "videoOffsetMs"); if (clock.audioOffsetMs !== undefined) safeInt(clock.audioOffsetMs, "audioOffsetMs");
  if (!Array.isArray(p.files)) fail("files must be an array");
  const paths = new Set<string>();
  p.files.forEach((file, index) => { validateSealedFile(file, `files[${index}]`); const f = file as SealedFile; contained(p.root as string, f.relativePath, "material file"); if (paths.has(f.relativePath)) fail("duplicate material path"); paths.add(f.relativePath); });
  const referenced = new Set<string>();
  const video = record(p.video, "video"); exact(video, ["file", "width", "height", "durationMs"], "video");
  validateMaterialRef(video.file, "video.file", paths, referenced); safeInt(video.width, "video.width", 1); safeInt(video.height, "video.height", 1); safeInt(video.durationMs, "video.durationMs");
  if (p.audio !== undefined) { const audio = record(p.audio, "audio"); exact(audio, ["file", "durationMs", "sampleRate", "channels"], "audio"); validateMaterialRef(audio.file, "audio.file", paths, referenced); safeInt(audio.durationMs, "audio.durationMs"); safeInt(audio.sampleRate, "audio.sampleRate", 1); safeInt(audio.channels, "audio.channels", 1); }
  const facts = record(p.facts, "facts"), collection = record(p.collection, "collection");
  exact(collection, Object.keys(factSchemas), "collection");
  optionalExact(facts, Object.keys(factSchemas), [], "facts");
  for (const key of Object.keys(factSchemas) as Array<keyof typeof factSchemas>) {
    const state = collection[key]; if (state !== "collected" && state !== "unavailable") fail(`collection.${key} has invalid state`);
    const ref = facts[key];
    if (state === "collected" && ref === undefined) fail(`collected ${key} is missing its typed reference`);
    if (state === "unavailable" && ref !== undefined) fail(`unavailable ${key} must not have a reference`);
    if (ref !== undefined) { const r = record(ref, `facts.${key}`); exact(r, ["file", "schema"], `facts.${key}`); validateMaterialRef(r.file, `facts.${key}.file`, paths, referenced); if (r.schema !== factSchemas[key]) fail(`facts.${key} has wrong schema`); }
  }
  if (!Array.isArray(p.warnings)) fail("warnings must be an array");
  p.warnings.forEach((warning, i) => { const w = record(warning, `warnings[${i}]`); optionalExact(w, ["code", "atMs"], ["code"], `warnings[${i}]`); string(w.code, "warning code"); if (w.atMs !== undefined) withinClock(w.atMs, clock.durationMs as number, "warning time"); });
  const safety = record(p.safety, "safety"); exact(safety, ["nativeRemoteDelivery", "safeForProcessing"], "safety"); if (safety.nativeRemoteDelivery !== "forbidden" || typeof safety.safeForProcessing !== "boolean") fail("invalid safety facts");
  // These fields are factual opaque documents owned by native/platform schemas.
  if (p.confirmedSource === undefined || p.invocation === undefined) fail("confirmed source and invocation facts must be explicit");
}
function validateMaterialRef(value: unknown, label: string, paths: Set<string>, referenced: Set<string>): void { relativePath(value, label); if (!paths.has(value)) fail(`${label} does not name a sealed material file`); if (referenced.has(value)) fail(`${label} duplicates another file reference`); referenced.add(value); }
export function withinClock(value: unknown, durationMs: number, label: string): asserts value is number { safeInt(value, label); if ((value as number) > durationMs) fail(`${label} exceeds capture duration`); }

export function validateMaterialsDocument(manifestInput: unknown, payloadBytes: Uint8Array): MaterialsDocument {
  const m = record(manifestInput, "materials manifest"); exact(m, ["schema", "key", "root", "leaseId", "payloadFile"], "materials manifest");
  if (m.schema !== "apparent.capture-materials/1") fail("unsupported materials manifest schema");
  validateMaterialsKey(m.key); absolutePath(m.root, "manifest root"); id(m.leaseId, "manifest leaseId"); validateSealedFile(m.payloadFile, "payloadFile");
  const payloadFile = m.payloadFile as SealedFile;
  if ((m.key as { revision?: unknown }).revision !== payloadFile.sha256) fail("materials revision does not bind exact payload bytes");
  if (payloadFile.identity.size !== payloadBytes.byteLength) fail("materials payload byte length differs from sealed identity");
  const parsed = parseDetached<unknown>(payloadBytes, payloadFile.sha256, "materials payload");
  validatePayload(parsed);
  const manifest = m as unknown as MaterialsManifest;
  validateMaterialEnvelopeIdentity(parsed as MaterialsPayload, manifest);
  if (parsed.files.some((file) => file.relativePath === manifest.payloadFile.relativePath)) fail("materials payload must be detached from its own file inventory");
  return { manifest, payload: parsed as MaterialsPayload };
}

export function validateStrokeFacts(fileEnvelope: SealedFile, bytes: Uint8Array, materials: MaterialsDocument): import("./types.ts").StrokeFacts {
  validateSealedFile(fileEnvelope, "stroke fact envelope");
  const ref = materials.payload.facts.acceptedStrokes;
  const listed = materials.payload.files.find((item) => item.relativePath === fileEnvelope.relativePath);
  if (!ref || ref.file !== fileEnvelope.relativePath || ref.schema !== "apparent.capture-strokes/1" || !listed || !isDeepStrictEqual(listed, fileEnvelope)) fail("stroke fact envelope is not the sealed accepted-strokes reference");
  if (fileEnvelope.identity.size !== bytes.byteLength) fail("stroke fact bytes differ from sealed size");
  const value = parseDetached<unknown>(bytes, fileEnvelope.sha256, "stroke facts");
  const s = record(value, "stroke facts"); exact(s, ["schema", "identity", "coordinates", "surfaces", "strokes"], "stroke facts");
  if (s.schema !== "apparent.capture-strokes/1" || s.coordinates !== "target-normalized-10000") fail("invalid stroke document schema or coordinates");
  const identity = record(s.identity, "stroke identity"); exact(identity, ["captureId", "materialsId"], "stroke identity");
  if (identity.captureId !== materials.payload.identity.captureId || identity.materialsId !== materials.payload.identity.materialsId) fail("stroke identity does not match materials");
  if (!Array.isArray(s.surfaces) || !Array.isArray(s.strokes)) fail("stroke surfaces and strokes must be arrays");
  const surfaceIds = new Set<string>();
  s.surfaces.forEach((item, i) => { const x = record(item, `surfaces[${i}]`); exact(x, ["surfaceId", "transformId", "width", "height", "target"], `surfaces[${i}]`); string(x.surfaceId, "surfaceId"); string(x.transformId, "transformId"); safeInt(x.width, "surface width", 1); safeInt(x.height, "surface height", 1); if (x.target !== "capture-rectangle" && x.target !== "whiteboard-canvas") fail("invalid surface target"); const key = `${x.surfaceId}\0${x.transformId}`; if (surfaceIds.has(key)) fail("duplicate surface transform"); surfaceIds.add(key); });
  const strokeIds = new Set<string>(), duration = materials.payload.clock.durationMs;
  s.strokes.forEach((item, i) => { const x = record(item, `strokes[${i}]`); exact(x, ["id", "surface", "surfaceId", "transformId", "startMs", "endMs", "acceptedAtMs", "points"], `strokes[${i}]`); id(x.id, "stroke id"); if (strokeIds.has(x.id)) fail("duplicate stroke id"); strokeIds.add(x.id); if (x.surface !== "screen" && x.surface !== "whiteboard") fail("invalid stroke surface"); string(x.surfaceId, "stroke surfaceId"); string(x.transformId, "stroke transformId"); if (!surfaceIds.has(`${x.surfaceId}\0${x.transformId}`)) fail("stroke references undeclared surface transform"); withinClock(x.startMs, duration, "stroke start"); withinClock(x.endMs, duration, "stroke end"); withinClock(x.acceptedAtMs, duration, "stroke acceptedAt"); if ((x.startMs as number) > (x.endMs as number) || (x.endMs as number) > (x.acceptedAtMs as number)) fail("invalid stroke time order"); if (!Array.isArray(x.points) || x.points.length === 0) fail("stroke must contain points"); let previous = x.startMs as number; x.points.forEach((point, j) => { const p = record(point, `stroke point ${j}`); exact(p, ["timeMs", "x", "y"], "stroke point"); withinClock(p.timeMs, duration, "point time"); safeInt(p.x, "point x"); safeInt(p.y, "point y"); if ((p.x as number) > 10000 || (p.y as number) > 10000) fail("stroke coordinates exceed normalized range"); if ((p.timeMs as number) < previous || (p.timeMs as number) < (x.startMs as number) || (p.timeMs as number) > (x.endMs as number)) fail("stroke point time is outside sorted stroke interval"); previous = p.timeMs as number; }); });
  return s as unknown as import("./types.ts").StrokeFacts;
}

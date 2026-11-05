import { isDeepStrictEqual } from "node:util";
import { absolutePath, assertProcessingAllowed, exact, fail, id, optionalExact, parseDetached, record, sameMaterials, string, validateMaterialsKey, validateSealedFile } from "./common.ts";
import type { InferenceEnvelope, InferencePayload, MaterialsDocument, SealedFile } from "./types.ts";
import { withinClock } from "./materials.ts";

export function validateInferenceEnvelope(value: unknown, receiptBytes: Uint8Array, materials: MaterialsDocument): asserts value is InferenceEnvelope {
  assertProcessingAllowed(materials);
  const e = record(value, "inference envelope"); exact(e, ["schema", "materials", "jobId", "receiptId", "root", "receiptFile", "payload"], "inference envelope");
  if (e.schema !== "apparent.transcript-result/1") fail("unsupported inference envelope"); validateMaterialsKey(e.materials); sameMaterials(e.materials as InferenceEnvelope["materials"], materials.manifest.key, "inference");
  id(e.jobId, "inference jobId"); id(e.receiptId, "inference receiptId"); absolutePath(e.root, "inference root"); validateSealedFile(e.receiptFile, "inference receiptFile");
  if ((e.receiptFile as SealedFile).identity.size !== receiptBytes.byteLength) fail("inference receipt byte length differs from sealed identity");
  const parsed = parseDetached<unknown>(receiptBytes, (e.receiptFile as SealedFile).sha256, "inference receipt payload");
  validatePayload(parsed, materials, e as unknown as InferenceEnvelope);
  if (!isDeepStrictEqual(e.payload, parsed)) fail("inference envelope payload differs from detached receipt bytes");
}

function validatePayload(value: unknown, materials: MaterialsDocument, envelope: InferenceEnvelope): asserts value is InferencePayload {
  const p = record(value, "inference payload"); optionalExact(p, ["schema", "materials", "jobId", "receiptId", "root", "clock", "state", "transcript", "warning"], ["schema", "materials", "jobId", "receiptId", "root", "clock", "state"], "inference payload");
  if (p.schema !== "apparent.transcript-result-payload/1" || p.clock !== "bundle") fail("unsupported inference payload schema or clock");
  validateMaterialsKey(p.materials); sameMaterials(p.materials as InferencePayload["materials"], materials.manifest.key, "inference payload");
  if (p.jobId !== envelope.jobId || p.receiptId !== envelope.receiptId || p.root !== envelope.root) fail("inference envelope/receipt identity mismatch");
  if (p.state === "unavailable") {
    if (p.transcript !== undefined || !p.warning || typeof p.warning !== "object" || (p.warning as Record<string, unknown>).code !== "transcript_unavailable" || typeof (p.warning as Record<string, unknown>).retryable !== "boolean") fail("unavailable inference requires warning and forbids transcript");
  } else if (p.state === "complete") {
    if (p.warning !== undefined || !p.transcript) fail("complete inference requires transcript and forbids unavailable warning");
    const t = record(p.transcript, "transcript"); exact(t, ["provider", "model", "text", "segments"], "transcript"); string(t.provider, "provider"); string(t.model, "model"); if (typeof t.text !== "string" || !Array.isArray(t.segments)) fail("invalid transcript contents");
    let previous = -1; t.segments.forEach((segment, i) => { const s = record(segment, `segments[${i}]`); if (s.endMs === undefined) exact(s, ["startMs", "text"], `segments[${i}]`); else exact(s, ["startMs", "endMs", "text"], `segments[${i}]`); withinClock(s.startMs, materials.payload.clock.durationMs, "segment start"); if ((s.startMs as number) < previous) fail("transcript segments are not sorted"); previous = s.startMs as number; if (s.endMs !== undefined) { withinClock(s.endMs, materials.payload.clock.durationMs, "segment end"); if ((s.endMs as number) < (s.startMs as number)) fail("transcript segment ends before it starts"); } if (typeof s.text !== "string") fail("segment text must be a string"); });
  } else fail("invalid inference state");
}

import { randomUUID } from "node:crypto";
import { join } from "node:path";
import type { MaterialsDocument } from "@apparent/engine-sdk/capture";
import type { Media2JobRequest } from "@apparent/engine-sdk/media2";
import type { VisualSelectionSample } from "../../selection/visual-keyframes.js";
import { selectVisualKeyframes } from "../../selection/visual-keyframes.js";
import type { StoredPlan, RecordCaptureProcessingSelection } from "./coordinator.js";

export interface StandardVisualPreferences {
  requestVideoProof: boolean;
  /** If absent, use the original invocation's persisted observedAtMs. */
  createdAtMs?: number;
  destination?: RecordCaptureProcessingSelection["admission"]["destination"];
}
export interface StandardVisualChoice extends StandardVisualPreferences { kind: "standard-visual" }
export interface StandardVisualPlan {
  schema: 2;
  kind: "standard-visual";
  policy: "record-visual/1";
  base: Omit<StoredPlan, "request" | "frames">;
  samples: Media2JobRequest[];
  finalJob: { jobId: string; outputDirectory: string };
}
const id = () => randomUUID().replaceAll("-", "");

/** Pure Record planning: all job identities and sampling times are stored before dispatch. */
export function planStandardVisual(materials: MaterialsDocument, base: StandardVisualPlan["base"], privateRoot: string): StandardVisualPlan {
  if (["interactions", "transcript", "acceptedStrokes"].some(key => materials.payload.collection[key as "interactions"] !== "unavailable"))
    throw new Error("Standard visual processing requires unavailable interaction, transcript and stroke collections; use explicit richer policy for collected facts");
  const { clock, video } = materials.payload;
  const span = Math.min(video.durationMs, clock.durationMs - clock.videoOffsetMs);
  if (!Number.isSafeInteger(span) || span < 0) throw new Error("Standard visual processing requires a valid retained video clock");
  const step = Math.max(220, Math.ceil((span + 1) / 480));
  const times: number[] = [];
  for (let t = 0; t <= span && times.length < 480; t += step) times.push(clock.videoOffsetMs + t);
  const samples: Media2JobRequest[] = [];
  for (let offset = 0; offset < times.length; offset += 48) {
    const jobId = id();
    samples.push({ schema: "apparent.retained-frame-request/1", materials: structuredClone(materials.manifest.key), leaseId: materials.manifest.leaseId,
      jobId, outputDirectory: join(privateRoot, `visual-samples-${jobId}`), requestedBundleMs: times.slice(offset, offset + 48),
      limits: { maxFrames: 48, maxEdge: 640, maxTotalPixels: 48 * 640 * 640, deadlineMs: 60_000 }, contactSheet: null });
  }
  const jobId = id();
  return { schema: 2, kind: "standard-visual", policy: "record-visual/1", base: structuredClone(base), samples,
    finalJob: { jobId, outputDirectory: join(privateRoot, `visual-frames-${jobId}`) } };
}

/** Final pixels retain whole-frame aspect; semantic captions stay outside the sheet. */
export function finishStandardVisual(plan: StandardVisualPlan, materials: MaterialsDocument, samples: readonly VisualSelectionSample[]): StoredPlan {
  const expected = plan.samples.flatMap(request => request.requestedBundleMs.map((timeMs, index) => ({ id: `${request.jobId}:${index + 1}`, timeMs })));
  if (samples.length !== expected.length || samples.some((sample, index) => sample.id !== expected[index]?.id || sample.timeMs !== expected[index]?.timeMs))
    throw new Error("Measured samples differ from the original standard sampling plan");
  const selected = selectVisualKeyframes(samples, { durationMs: materials.payload.clock.durationMs });
  if (!selected.length) throw new Error("Standard visual selection requires measured frames");
  const count = selected.length;
  const { width, height } = materials.payload.video;
  // At most 24M frame pixels and 4M sheet pixels, below native's 64M budget.
  const edge = Math.min(4096, Math.floor(Math.sqrt(24_000_000 / count)));
  const scale = Math.min(1, edge / Math.max(width, height));
  const tileScale = Math.min(1, 320 / Math.max(width, height));
  const tileWidth = Math.max(1, Math.round(width * tileScale)), tileHeight = Math.max(1, Math.round(height * tileScale));
  const columns = Math.min(3, count), rows = Math.ceil(count / columns);
  const request: Media2JobRequest = { schema: "apparent.retained-frame-request/1", materials: structuredClone(materials.manifest.key), leaseId: materials.manifest.leaseId,
    ...plan.finalJob, requestedBundleMs: selected.map(frame => frame.timeMs),
    limits: { maxFrames: count, maxEdge: edge, maxTotalPixels: 28_000_000, deadlineMs: 60_000 },
    contactSheet: { schema: "apparent.contact-sheet-layout/1", width: columns * tileWidth, height: rows * tileHeight,
      backgroundRgb: [0, 0, 0], fit: "stretch", sampling: "legacy-bilinear", clipping: "reject",
      tiles: selected.map((_, index) => ({ requestIndex: index + 1, x: index % columns * tileWidth, y: Math.floor(index / columns) * tileHeight, width: tileWidth, height: tileHeight })) } };
  const warnings = [...(plan.base.processingWarnings ?? [])];
  if (plan.samples.flatMap(sample => sample.requestedBundleMs).some((time, index, times) => index > 0 && time - times[index - 1]! > 220))
    warnings.push({ kind: "imageQuality", message: "Visual sampling was coarsened to the 480-frame processing bound." });
  if (scale < 1) warnings.push({ kind: "imageQuality", message: "Selected whole frames were downscaled to the bounded image budget; the original video is retained." });
  return { ...structuredClone(plan.base), schema: 1, request,
    frames: selected.map((frame, index) => ({ requestIndex: index + 1, index: index + 1, timeMs: frame.timeMs,
      bundlePath: `frames/frame-${String(index + 1).padStart(2, "0")}.png`, canonicalReason: frame.reason, event: frame.event })),
    ...(warnings.length ? { processingWarnings: warnings } : {}) };
}

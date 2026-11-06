/** Pure visual keyframe policy over caller-supplied, measured 64×36 luma thumbnails. */

export const VISUAL_THUMBNAIL_WIDTH = 64;
export const VISUAL_THUMBNAIL_HEIGHT = 36;
export const VISUAL_THUMBNAIL_PIXELS = VISUAL_THUMBNAIL_WIDTH * VISUAL_THUMBNAIL_HEIGHT;
export const MAX_VISUAL_SAMPLES = 480;
export const MAX_VISUAL_SOURCE_DIMENSION = 32_768;
export const BASE_VISUAL_KEYFRAMES = 6;
export const MAX_VISUAL_KEYFRAMES = 12;

const FRAME_GAP_MS = 750;
const SCENE_CHANGE_THRESHOLD = Math.fround(0.08);
const CUE_NOVELTY = Math.fround(0.03);
const COVERAGE_MS = Math.fround(35_000);

export interface VisualSelectionSample {
  /** Stable Record-owned identity for this already measured sample. */
  id: string;
  /** Bundle-clock sample time in milliseconds. */
  timeMs: number;
  /** Dimensions of the decoded source plane used to produce the thumbnail. */
  width: number;
  height: number;
  /** Exactly 2,304 row-major luma values in [0, 1], already measured by the caller. */
  thumbnail: readonly number[];
}

export type VisualKeyframeReason = "initial" | "scene_change";

export interface SelectedVisualKeyframe {
  sampleId: string;
  timeMs: number;
  span: { startMs: number; endMs: number };
  reason: VisualKeyframeReason;
  score: number;
  event: "initial view" | "scene changed";
}

interface Candidate {
  sampleIndex: number;
  spanStartMs: number;
  spanEndMs: number;
  score: number;
  reason: VisualKeyframeReason;
  event: "initial view" | "scene changed";
}

function f32(value: number): number { return Math.fround(value); }

function validInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function validate(samples: readonly VisualSelectionSample[], durationMs: number): void {
  if (!Array.isArray(samples) || samples.length > MAX_VISUAL_SAMPLES)
    throw new RangeError(`samples must contain at most ${MAX_VISUAL_SAMPLES} entries`);
  if (!validInteger(durationMs)) throw new RangeError("durationMs must be a non-negative safe integer");
  const ids = new Set<string>();
  let dimensions: string | undefined;
  let previousTime = -1;
  for (const [index, sample] of samples.entries()) {
    if (!sample || typeof sample !== "object") throw new TypeError(`sample ${index} must be an object`);
    if (typeof sample.id !== "string" || !sample.id.trim() || ids.has(sample.id))
      throw new TypeError("sample IDs must be non-empty and unique");
    ids.add(sample.id);
    if (!validInteger(sample.timeMs) || sample.timeMs > durationMs || sample.timeMs < previousTime)
      throw new RangeError("sample times must be ordered non-negative safe integers within durationMs");
    previousTime = sample.timeMs;
    if (!Number.isSafeInteger(sample.width) || sample.width <= 0 || sample.width > MAX_VISUAL_SOURCE_DIMENSION ||
        !Number.isSafeInteger(sample.height) || sample.height <= 0 || sample.height > MAX_VISUAL_SOURCE_DIMENSION ||
        !Number.isSafeInteger(sample.width * sample.height))
      throw new RangeError(`sample dimensions must be positive bounded integers no larger than ${MAX_VISUAL_SOURCE_DIMENSION}`);
    const size = `${sample.width}x${sample.height}`;
    if (dimensions !== undefined && dimensions !== size)
      throw new RangeError("all visual samples must use the same decoded plane dimensions");
    dimensions = size;
    if (!Array.isArray(sample.thumbnail) || sample.thumbnail.length !== VISUAL_THUMBNAIL_PIXELS)
      throw new RangeError(`sample thumbnail must contain exactly ${VISUAL_THUMBNAIL_PIXELS} luma values`);
    for (const value of sample.thumbnail) {
      if (!Number.isFinite(value) || value < 0 || value > 1)
        throw new RangeError("thumbnail luma values must be finite numbers in [0, 1]");
      if (!Number.isFinite(f32(value))) throw new RangeError("thumbnail luma value is outside float32 range");
    }
  }
}

/** Matches the native duration budget: six through 39,999 ms, then one per 20 seconds, capped at twelve. */
export function visualKeyframeBudget(durationMs: number): number {
  if (!validInteger(durationMs)) throw new RangeError("durationMs must be a non-negative safe integer");
  return Math.min(BASE_VISUAL_KEYFRAMES + Math.floor(Math.max(durationMs - 20_000, 0) / 20_000), MAX_VISUAL_KEYFRAMES);
}

function thumbnailDiff(a: readonly number[], b: readonly number[]): number {
  let sum = f32(0);
  for (let index = 0; index < VISUAL_THUMBNAIL_PIXELS; index++) {
    const difference = f32(Math.abs(f32(f32(a[index]!) - f32(b[index]!))));
    sum = f32(sum + difference);
  }
  return Math.min(1, Math.max(0, f32(sum / f32(VISUAL_THUMBNAIL_PIXELS))));
}

/** Native global luma plus 1.4× the fourth-largest 8×6 block difference. */
function sceneTransitionDiff(a: readonly number[], b: readonly number[]): number {
  const global = thumbnailDiff(a, b);
  const top = [0, 0, 0, 0];
  const blockWidth = VISUAL_THUMBNAIL_WIDTH / 8;
  const blockHeight = VISUAL_THUMBNAIL_HEIGHT / 6;
  for (let blockY = 0; blockY < 6; blockY++) {
    for (let blockX = 0; blockX < 8; blockX++) {
      let sum = f32(0);
      for (let y = blockY * blockHeight; y < (blockY + 1) * blockHeight; y++) {
        for (let x = blockX * blockWidth; x < (blockX + 1) * blockWidth; x++) {
          const index = y * VISUAL_THUMBNAIL_WIDTH + x;
          const difference = f32(Math.abs(f32(f32(a[index]!) - f32(b[index]!))));
          sum = f32(sum + difference);
        }
      }
      const blockDiff = f32(sum / f32(blockWidth * blockHeight));
      if (blockDiff > top[3]!) {
        top[3] = blockDiff;
        top.sort((left, right) => right - left);
      }
    }
  }
  const local = Math.min(1, Math.max(0, f32(top[3]! * f32(1.4))));
  return Math.max(global, local);
}

function candidateAnchor(candidate: Candidate, samples: readonly VisualSelectionSample[]): number {
  return Math.min(candidate.spanStartMs, samples[candidate.sampleIndex]!.timeMs);
}

function admissible(candidate: Candidate, selected: readonly Candidate[], samples: readonly VisualSelectionSample[]): boolean {
  const time = samples[candidate.sampleIndex]!.timeMs;
  return selected.every((other) => {
    if (Math.abs(samples[other.sampleIndex]!.timeMs - time) < FRAME_GAP_MS) return false;
    return sceneTransitionDiff(samples[other.sampleIndex]!.thumbnail, samples[candidate.sampleIndex]!.thumbnail) >= CUE_NOVELTY;
  });
}

function f32Ratio(numerator: number, denominator: number): number {
  return f32(f32(numerator) / f32(denominator));
}

/** Visual-only counterpart of the donor's empty-transcript long-stretch coverage pass. */
function coverLongStretches(
  selected: Candidate[], pending: Array<Candidate | undefined>, maxKeyframes: number,
  samples: readonly VisualSelectionSample[],
): void {
  if (selected.length === 0) {
    const firstIndex = pending.findIndex((candidate) => candidate && admissible(candidate, selected, samples));
    if (firstIndex >= 0) {
      selected.push(pending[firstIndex]!);
      pending[firstIndex] = undefined;
    }
  }
  const lastTime = samples.at(-1)?.timeMs ?? 0;
  const end = lastTime + 1;
  const size = (start: number, finish: number) => f32Ratio(f32(finish - start), COVERAGE_MS);
  const stretches = (anchors: number[], seen: boolean): Array<[number, number]> => {
    const bounds = [0, ...anchors.slice(Number(!seen)), end];
    return bounds.slice(0, -1).map((bound, index) => [bound, bounds[index + 1]!] as [number, number]);
  };
  const worstWithin = (pieces: Array<[number, number]>, bounds: [number, number]) =>
    pieces.filter(([start, finish]) => start >= bounds[0] && finish <= bounds[1])
      .reduce((worst, [start, finish]) => Math.max(worst, size(start, finish)), 0);

  const rejected = new Set<number>();
  while (selected.length < maxKeyframes) {
    const anchors = selected.map((candidate) => candidateAnchor(candidate, samples)).sort((a, b) => a - b);
    const long = stretches(anchors, false).map((stretch) => [size(...stretch), stretch] as const)
      .filter(([stretchSize]) => stretchSize > 1).sort((a, b) => b[0] - a[0]);
    let pick: number | undefined;
    for (const [whole, bounds] of long) {
      const options: Array<{ uneven: boolean; worst: number; seen: number; rank: number }> = [];
      for (let rank = 0; rank < pending.length; rank++) {
        const candidate = pending[rank];
        if (!candidate || rejected.has(rank)) continue;
        const anchor = candidateAnchor(candidate, samples);
        if (anchors.includes(anchor) || anchor < bounds[0] || anchor >= bounds[1]) continue;
        const insertion = anchors.findIndex((value) => value > anchor);
        const withAnchor = anchors.slice();
        withAnchor.splice(insertion < 0 ? withAnchor.length : insertion, 0, anchor);
        const worst = worstWithin(stretches(withAnchor, false), bounds);
        const gap = stretches(anchors, true).find(([start, finish]) => start <= anchor && anchor < finish) ?? bounds;
        const seen = worstWithin(stretches(withAnchor, true), gap);
        if (worst < whole) {
          const uneven = worst > f32(f32(0.75) * f32(whole)) || seen > f32(f32(0.75) * size(...gap));
          options.push({ uneven, worst, seen, rank });
        }
      }
      options.sort((left, right) => Number(left.uneven) - Number(right.uneven) ||
        (left.uneven ? left.worst - right.worst || left.seen - right.seen : left.rank - right.rank));
      for (const option of options) {
        if (admissible(pending[option.rank]!, selected, samples)) {
          pick = option.rank;
          break;
        }
        rejected.add(option.rank);
      }
      if (pick !== undefined) break;
    }
    if (pick === undefined) break;
    selected.push(pending[pick]!);
    pending[pick] = undefined;
  }
}

/**
 * Selects scene changes and an initial static fallback from actual caller-supplied measurements.
 * This function neither decodes images nor asserts that samples came from a native candidate API.
 */
export function selectVisualKeyframes(
  samples: readonly VisualSelectionSample[],
  options: { durationMs: number; maxKeyframes?: number },
): SelectedVisualKeyframe[] {
  if (!options || typeof options !== "object") throw new TypeError("options are required");
  validate(samples, options.durationMs);
  const maxKeyframes = options.maxKeyframes === undefined ? visualKeyframeBudget(options.durationMs) : options.maxKeyframes;
  if (!Number.isSafeInteger(maxKeyframes) || maxKeyframes < 0 || maxKeyframes > MAX_VISUAL_KEYFRAMES)
    throw new RangeError(`maxKeyframes must be an integer from 0 through ${MAX_VISUAL_KEYFRAMES}`);
  if (!samples.length || maxKeyframes === 0) return [];

  const candidates: Candidate[] = [];
  let transition: { sampleIndex: number; startMs: number; maxDiff: number } | undefined;
  for (let index = 0; index + 1 < samples.length; index++) {
    const delta = sceneTransitionDiff(samples[index]!.thumbnail, samples[index + 1]!.thumbnail);
    if (delta > SCENE_CHANGE_THRESHOLD) {
      transition = {
        sampleIndex: index + 1,
        startMs: transition?.startMs ?? samples[index]!.timeMs,
        maxDiff: transition ? Math.max(transition.maxDiff, delta) : delta,
      };
    } else if (transition) {
      const score = f32(f32(40) + f32(f32(40) * f32(transition.maxDiff)));
      candidates.push({ sampleIndex: transition.sampleIndex, spanStartMs: transition.startMs,
        spanEndMs: samples[transition.sampleIndex]!.timeMs, score, reason: "scene_change", event: "scene changed" });
      transition = undefined;
    }
  }
  if (transition) {
    const score = f32(f32(40) + f32(f32(40) * f32(transition.maxDiff)));
    candidates.push({ sampleIndex: transition.sampleIndex, spanStartMs: transition.startMs,
      spanEndMs: samples[transition.sampleIndex]!.timeMs, score, reason: "scene_change", event: "scene changed" });
  }
  if (!candidates.length) {
    const sampleIndex = Math.floor(samples.length / 2);
    const timeMs = samples[sampleIndex]!.timeMs;
    candidates.push({ sampleIndex, spanStartMs: timeMs, spanEndMs: timeMs, score: 0, reason: "initial", event: "initial view" });
  }
  candidates.sort((left, right) => right.score - left.score ||
    samples[left.sampleIndex]!.timeMs - samples[right.sampleIndex]!.timeMs);

  const selected: Candidate[] = [];
  const pending: Array<Candidate | undefined> = candidates.slice();
  coverLongStretches(selected, pending, maxKeyframes, samples);
  for (let index = 0; index < pending.length && selected.length < Math.min(maxKeyframes, BASE_VISUAL_KEYFRAMES); index++) {
    const candidate = pending[index];
    if (candidate && admissible(candidate, selected, samples)) {
      selected.push(candidate);
      pending[index] = undefined;
    }
  }
  selected.sort((left, right) => samples[left.sampleIndex]!.timeMs - samples[right.sampleIndex]!.timeMs);
  return selected.map((candidate) => ({
    sampleId: samples[candidate.sampleIndex]!.id,
    timeMs: samples[candidate.sampleIndex]!.timeMs,
    span: { startMs: candidate.spanStartMs, endMs: candidate.spanEndMs },
    reason: candidate.reason,
    score: candidate.score,
    event: candidate.event,
  }));
}

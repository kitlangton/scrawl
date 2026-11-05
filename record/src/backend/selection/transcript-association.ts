/** Record policy for assigning an existing transcript to Engine-selected frames. */

export interface TimedTranscriptUnit {
  startMs: number;
  endMs: number;
  text: string;
}

export interface TranscriptFacts {
  /** When word timings exist they are authoritative; otherwise timed segments are used. */
  words?: TimedTranscriptUnit[];
  segments: TimedTranscriptUnit[];
  /** Full text is retained when a producer has no timed units. */
  text?: string;
}

export interface SelectedFrameReference {
  /** Stable, unique ID supplied by the existing selector. */
  id: string;
  /** Bundle-clock timestamp of the selected frame. */
  timeMs: number;
  /** Start of the action/cue represented by this frame; defaults to timeMs. */
  anchorTimeMs?: number;
  /** Absolute native candidate reference. This policy never opens or creates the image. */
  absolutePath: string;
}

export interface AssociatedFrame extends SelectedFrameReference {
  said?: string;
}

const clauseGapMs = 1_500;

function validTime(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function endsClause(text: string, words: number): boolean {
  const trimmed = text.replace(/["'”’\)\]]+$/u, "");
  return /[.!?;:]$/u.test(trimmed) || (words >= 3 && trimmed.endsWith(","));
}

function wordCount(text: string): number {
  return Math.max(1, text.split(/\p{White_Space}+/u).filter((word) => /[\p{Alphabetic}\p{N}]/u.test(word)).length);
}

// Match Rust's Unicode White_Space trimming, which differs from JS trim for
// NEL and BOM. Alphabetic also includes marks outside the Letter category.
const trimTranscript = (text: string) => text.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");

function isAttachedPunctuation(text: string): boolean {
  return /^[,.!?;:'’\)\]”]/u.test(text);
}

function isOpeningPunctuation(text: string): boolean {
  return /[(\[“]$/u.test(text);
}

/**
 * Assigns transcript units once, in time order, to already selected frames. Clause starts
 * follow punctuation, measured gaps, or transcript segment boundaries just like Engine's
 * existing keyframe text policy. This operation does not select frames or infer visual change.
 */
export function associateTranscriptWithSelectedFrames(
  frames: readonly SelectedFrameReference[],
  transcript: TranscriptFacts,
): AssociatedFrame[] {
  const ids = new Set<string>();
  for (const [index, frame] of frames.entries()) {
    if (!frame.id.trim() || ids.has(frame.id)) throw new TypeError("frame IDs must be non-empty and unique");
    ids.add(frame.id);
    if (!validTime(frame.timeMs) || (frame.anchorTimeMs !== undefined && !validTime(frame.anchorTimeMs)))
      throw new RangeError("frame timestamps must be non-negative safe integers");
    if (!frame.absolutePath.startsWith("/") || frame.absolutePath.includes("\\") ||
      frame.absolutePath.split("/").some((part) => part === "." || part === ".."))
      throw new TypeError("frame reference must be an absolute native path");
    if (index > 0 && frame.timeMs < frames[index - 1]!.timeMs)
      throw new RangeError("selected frames must be in timestamp order");
  }
  if (!frames.length) return [];

  const words = transcript.words ?? [];
  const source = words.length ? words : transcript.segments;
  const units = source.filter((unit) => trimTranscript(unit.text)).map((unit) => {
    if (!validTime(unit.startMs) || !validTime(unit.endMs) || unit.endMs < unit.startMs)
      throw new RangeError("transcript unit timestamps must be ordered non-negative safe integers");
    return unit;
  }).slice().sort((a, b) => a.startMs - b.startMs);
  for (const segment of transcript.segments) {
    if (!validTime(segment.startMs) || !validTime(segment.endMs) || segment.endMs < segment.startMs)
      throw new RangeError("transcript segment timestamps must be ordered non-negative safe integers");
  }

  const clauses: Array<{ startMs: number; endMs: number }> = [];
  let previous: TimedTranscriptUnit | undefined;
  let clauseWords = 0;
  for (const unit of units) {
    const segmentStart = transcript.segments.some((segment) => Math.abs(segment.startMs - unit.startMs) <= 20);
    const startsClause = !previous || endsClause(previous.text, clauseWords) ||
      unit.startMs - previous.endMs >= clauseGapMs || segmentStart;
    const last = clauses.at(-1);
    if (!startsClause && last) {
      last.endMs = Math.max(last.endMs, unit.endMs);
      clauseWords += wordCount(unit.text);
    } else {
      clauses.push({ startMs: unit.startMs, endMs: unit.endMs });
      clauseWords = wordCount(unit.text);
    }
    previous = unit;
  }

  const starts: number[] = [];
  for (const frame of frames) {
    const anchor = frame.anchorTimeMs ?? frame.timeMs;
    let start = clauses.find((clause) => clause.startMs <= anchor && anchor <= clause.endMs)?.startMs ?? anchor;
    const previous = starts.at(-1);
    if (previous !== undefined && start <= previous) start = Math.max(anchor, previous + 1);
    starts.push(start);
  }

  // Preserve supplied selector order; the selector owns ordering and stable frame identity.
  const result: AssociatedFrame[] = frames.map((frame) => ({ ...frame }));
  const said = Array.from({ length: frames.length }, () => "");
  for (const unit of units) {
    const text = trimTranscript(unit.text);
    let index = 0;
    for (let i = 0; i < starts.length; i++) if (starts[i]! <= unit.startMs + 50) index = i;
    if (isAttachedPunctuation(text) && !said[index] && index > 0) index--;
    const open = isOpeningPunctuation(said[index]!);
    if (said[index] && !isAttachedPunctuation(text) && !open) said[index] += " ";
    said[index] += text;
  }
  if (said.every((text) => !text) && transcript.text && trimTranscript(transcript.text)) said[0] = trimTranscript(transcript.text);
  return result.map((frame, index) => ({ ...frame, ...(said[index] ? { said: said[index] } : {}) }));
}

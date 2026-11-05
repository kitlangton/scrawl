import { isAbsolute, relative, resolve, sep } from "node:path";

/** Record-owned assembly of semantic handoff text from facts supplied by Engine. */

export interface TranscriptSegment {
  startMs: number;
  text: string;
}

export interface SelectedFrame {
  /** Existing Engine selection index, kept stable in text and image filenames. */
  index: number;
  timeMs: number;
  absolutePath: string;
  text?: string;
}

export interface ProcessingInput {
  durationMs: number;
  bundlePath: string;
  transcript: { text: string; segments: TranscriptSegment[] };
  frames: SelectedFrame[];
  notes?: Array<{ timeMs: number; text: string }>;
  videoPath?: string;
}

export interface ProcessingOutput {
  summaryMarkdown: string;
}

/** Structural subset of the published @apparent/engine-sdk/media client. */
export interface PublishedMediaClient {
  decode(input: {
    jobId: string;
    source: {
      bundleId: string; bundlePath: string; videoPath: string;
      videoOffsetMs: number;
      identity: { device: string; inode: string; size: number; modifiedNs: string };
    };
    requestedBundleMs: number[];
    outputDirectory: string;
    limits: { maxFrames: number; maxEdge: number; maxTotalPixels: number; deadlineMs: number };
  }): Promise<{ jobId: string; state: "complete"; frames: Array<{
    requestIndex: number; requestedBundleMs: number; effectiveVideoMs: number;
    path: string; width: number; height: number; byteLength: number; sha256: string;
  }> }>;
}

export interface DecodeSelection {
  /** Stable IDs and timestamps come from Record's existing policy selector. */
  frames: Array<{ index: number; timeMs: number; text?: string }>;
  jobId: string;
  source: {
    bundleId: string; bundlePath: string; videoPath: string; videoOffsetMs: number;
    identity: { device: string; inode: string; size: number; modifiedNs: string };
  };
  /** Caller creates a fresh, private job directory under a private parent. */
  outputDirectory: string;
  limits: { maxFrames: number; maxEdge: number; maxTotalPixels: number; deadlineMs: number };
}

/** Decode Record-selected bundle-clock timestamps through the published media/1 SDK contract. */
export async function decodeSelectedFrames(
  media: PublishedMediaClient,
  selection: DecodeSelection,
): Promise<SelectedFrame[]> {
  if (!selection.frames.length) return [];
  if (!isAbsolute(selection.outputDirectory) || resolve(selection.outputDirectory) !== selection.outputDirectory)
    throw new TypeError("outputDirectory must be an absolute normalized path");
  if (selection.frames.length > selection.limits.maxFrames)
    throw new RangeError("selected frames exceed the requested decode limit");
  const result = await media.decode({
    jobId: selection.jobId,
    source: selection.source,
    requestedBundleMs: selection.frames.map((frame) => frame.timeMs),
    outputDirectory: selection.outputDirectory,
    limits: selection.limits,
  });
  if (result.jobId !== selection.jobId || result.frames.length !== selection.frames.length)
    throw new Error("media SDK returned a mismatched decode inventory");
  return selection.frames.map((selected, i) => {
    const decoded = result.frames[i];
    const fromOutput = relative(selection.outputDirectory, decoded.path);
    if (decoded.requestIndex !== i + 1 || decoded.requestedBundleMs !== selected.timeMs ||
      !isAbsolute(decoded.path) || resolve(decoded.path) !== decoded.path ||
      fromOutput === "" || fromOutput === ".." || fromOutput.startsWith(`..${sep}`) || isAbsolute(fromOutput))
      throw new Error("media SDK returned an invalid ordered frame inventory");
    return { index: selected.index, timeMs: selected.timeMs, absolutePath: decoded.path, text: selected.text };
  });
}

function clock(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function validate(input: ProcessingInput): void {
  if (!Number.isSafeInteger(input.durationMs) || input.durationMs < 0)
    throw new RangeError("durationMs must be a non-negative safe integer");
  if (!isAbsolute(input.bundlePath) || resolve(input.bundlePath) !== input.bundlePath)
    throw new TypeError("bundlePath must be absolute and normalized");
  let previous = -1;
  for (const frame of input.frames) {
    if (!Number.isSafeInteger(frame.index) || frame.index < 1 || frame.index <= previous)
      throw new TypeError("frame indices must be positive and strictly increasing");
    if (!Number.isSafeInteger(frame.timeMs) || frame.timeMs < 0 || frame.timeMs > input.durationMs)
      throw new RangeError("frame time must be within the recording duration");
    if (!isAbsolute(frame.absolutePath) || resolve(frame.absolutePath) !== frame.absolutePath)
      throw new TypeError("frame paths must be absolute and normalized");
    previous = frame.index;
  }
  for (const segment of input.transcript.segments) {
    if (!Number.isSafeInteger(segment.startMs) || segment.startMs < 0 || segment.startMs > input.durationMs)
      throw new RangeError("transcript time must be within the recording duration");
  }
  for (const note of input.notes ?? []) {
    if (!Number.isSafeInteger(note.timeMs) || note.timeMs < 0 || note.timeMs > input.durationMs)
      throw new RangeError("note time must be within the recording duration");
  }
}

/**
 * Assemble existing Engine outputs into Record's plain-text handoff format.
 * This function selects no frames and invents no transcript, image, or media facts.
 */
export function assembleProcessingOutput(input: ProcessingInput): ProcessingOutput {
  validate(input);
  const lines = [`[apparent ${clock(input.durationMs)} · ${input.bundlePath}]`];
  if (input.frames.length) {
    lines.push("Read the frame images with this text; the images show visual details the words may not capture.");
    for (const frame of input.frames) {
      const said = frame.text?.trim();
      lines.push(said ? `- #${frame.index} (${clock(frame.timeMs)}) “${said}”` : `- #${frame.index} (${clock(frame.timeMs)})`);
      lines.push(`  ${frame.absolutePath}`);
    }
  }
  const timeline = [
    ...input.transcript.segments.map((segment) => ({ timeMs: segment.startMs, text: `“${segment.text.trim()}”` })),
    ...(input.notes ?? []).map((note) => ({ timeMs: note.timeMs, text: `Typed note: ${note.text.trim()}` })),
  ].filter((item) => item.text.length > 2).sort((a, b) => a.timeMs - b.timeMs);
  if (timeline.length) {
    for (const item of timeline) lines.push(`· ${clock(item.timeMs)} ${item.text}`);
  } else if (!input.transcript.segments.length && input.transcript.text.trim()) {
    lines.push(`“${input.transcript.text.trim()}”`);
  }
  if (input.videoPath) {
    if (!isAbsolute(input.videoPath) || resolve(input.videoPath) !== input.videoPath)
      throw new TypeError("videoPath must be absolute and normalized");
    lines.push(`Video: ${input.videoPath}`);
  }
  lines.push(`Bundle: ${input.bundlePath}`);
  return { summaryMarkdown: `${lines.join("\n")}\n` };
}


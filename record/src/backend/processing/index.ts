import { audioInitialGapText, type AudioInitialGapWarning } from "@apparent/record/capabilities";
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
  /** Existing Engine summary facts for this selected keyframe. */
  event?: string;
  reason?: "click" | "flag" | "other";
  screenName?: string;
  markId?: string;
}

/** Preformatted semantic context supplied for the corresponding selected frame. */
export interface SemanticFrameText {
  frameIndex: number;
  head: string;
  lines: string[];
}

export interface RecordTarget {
  app?: string;
  /** Manifest context only; never rendered in summary or clipboard handoff. */
  windowTitle?: string;
}

export interface RecordDestination {
  session: string;
  title?: string;
  url?: string;
}

export interface RecordMark {
  id: string;
  shape: "circle" | "underline" | "line" | "arrow" | "tap" | "pin" | "mark" | "flag";
  pin?: number;
  framePath?: string;
}

export type RecordInteraction =
  | { kind: "click"; timeMs: number }
  | { kind: "appEvent"; timeMs: number; label: string }
  | { kind: "note"; timeMs: number; pin: number; text: string };

export interface ContextChange {
  timeMs: number;
  key: string;
  value?: string;
}

export type ProcessingWarning =
  | AudioInitialGapWarning
  | { kind: "videoOnlyInteractionTelemetry" }
  | { kind: "imageQuality"; message: string }
  | { kind: "noMicrophoneAudio" }
  | { kind: "noTranscript"; message: string }
  | { kind: "noWordsTranscribed" };

export interface RecordDevice {
  name: string;
  app?: string;
  appShortVersion?: string;
  appVersion?: string;
}

export interface ProcessingInput {
  durationMs: number;
  bundlePath: string;
  target?: RecordTarget;
  destination?: RecordDestination;
  device?: RecordDevice;
  screenAtStart?: string;
  context?: Record<string, string>;
  contextChanges?: ContextChange[];
  transcript: { text: string; segments: TranscriptSegment[] };
  frames: SelectedFrame[];
  /** Caller-supplied rich semantic chunks; these are not authenticated bundle facts. */
  semanticFrameText?: { mode: "rich-context"; frames: SemanticFrameText[] };
  marks?: RecordMark[];
  events?: RecordInteraction[];
  /** Compatibility input retained for callers of the first processing slice. */
  notes?: Array<{ timeMs: number; text: string; pin?: number }>;
  note?: string;
  warnings?: ProcessingWarning[];
  autocrop?: { label: string; fullFramesDirectory: string };
  contactSheetPath?: string;
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

function contextValue(raw: string): string {
  const compact = raw.split(/\s+/u).filter(Boolean).join(" ");
  const chars = [...compact];
  return chars.length <= 60 ? compact : `${chars.slice(0, 59).join("").trimEnd()}…`;
}

function quoteLabel(label: string): string {
  return label.includes("“") || label.startsWith("<") ? label : `“${label}”`;
}

function targetLabel(input: ProcessingInput): string {
  if (input.device) {
    let target = input.device.name;
    if (input.device.app) {
      target += ` · ${input.device.app}`;
      const short = input.device.appShortVersion?.trim();
      const build = input.device.appVersion?.trim();
      if (short && build && short !== build) target += ` ${short} (${build})`;
      else if (short) target += ` ${short}`;
      else if (build) target += ` (${build})`;
    }
    return target;
  }
  return input.target?.app || "Display";
}

interface TimelineItem { timeMs: number; text: string }

function needsTranscriptTimeline(input: ProcessingInput): boolean {
  if (!input.frames.length) return true;
  const compact = (value: string) => value.split(/\s+/u).filter(Boolean).join(" ");
  const annotated = compact(input.frames.map(frame => frame.text ?? "").join(" "));
  const supplied = compact(input.transcript.segments.length
    ? input.transcript.segments.map(segment => segment.text).join(" ") : input.transcript.text);
  return supplied.length > 0 && annotated !== supplied;
}

function typedNote(note: Extract<RecordInteraction, { kind: "note" }>, marks: RecordMark[]): string {
  const frame = marks.find((mark) => mark.pin === note.pin)?.framePath;
  const label = `Typed note · pin ${note.pin}${frame ? ` · ${frame}` : ""}:`;
  const noteLines = note.text.split(/\r?\n/u);
  if (noteLines.at(-1) === "") noteLines.pop();
  return `${label}${noteLines.map((line) => `\n    ${line}`).join("")}`;
}

function legacyTimeline(input: ProcessingInput): TimelineItem[] {
  const items: TimelineItem[] = [];
  for (const event of input.events ?? []) {
    if (event.kind === "appEvent") items.push({ timeMs: event.timeMs, text: `app: ${event.label}` });
    if (event.kind === "note") items.push({ timeMs: event.timeMs, text: typedNote(event, input.marks ?? []) });
  }
  for (const note of input.notes ?? []) {
    if (note.pin !== undefined) {
      items.push({ timeMs: note.timeMs, text: typedNote({ kind: "note", timeMs: note.timeMs, pin: note.pin, text: note.text }, input.marks ?? []) });
    } else if (note.text.trim()) {
      items.push({ timeMs: note.timeMs, text: `Typed note: ${note.text.trim()}` });
    }
  }
  for (const change of input.contextChanges ?? []) {
    const key = contextValue(change.key);
    items.push({ timeMs: change.timeMs, text: change.value === undefined ? `${key} cleared` : `${key} → ${contextValue(change.value)}` });
  }
  if (needsTranscriptTimeline(input)) {
    for (const segment of input.transcript.segments)
      items.push({ timeMs: segment.startMs, text: `“${segment.text}”` });
    if (!input.transcript.segments.length && input.transcript.text.trim())
      items.push({ timeMs: 0, text: `“${input.transcript.text.trim()}”` });
  }
  return items.sort((a, b) => a.timeMs - b.timeMs);
}

function validate(input: ProcessingInput): void {
  if (!Number.isSafeInteger(input.durationMs) || input.durationMs < 0)
    throw new RangeError("durationMs must be a non-negative safe integer");
  if (!isAbsolute(input.bundlePath) || resolve(input.bundlePath) !== input.bundlePath)
    throw new TypeError("bundlePath must be absolute and normalized");
  let previous = -1;
  let previousTime = -1;
  for (const frame of input.frames) {
    if (!Number.isSafeInteger(frame.index) || frame.index < 1 || frame.index <= previous)
      throw new TypeError("frame indices must be positive and strictly increasing");
    if (!Number.isSafeInteger(frame.timeMs) || frame.timeMs < 0 || frame.timeMs > input.durationMs)
      throw new RangeError("frame time must be within the recording duration");
    if (frame.timeMs < previousTime) throw new TypeError("frames must be in time order");
    if (!isAbsolute(frame.absolutePath) || resolve(frame.absolutePath) !== frame.absolutePath)
      throw new TypeError("frame paths must be absolute and normalized");
    previous = frame.index;
    previousTime = frame.timeMs;
  }
  if (input.semanticFrameText !== undefined) {
    const rich = input.semanticFrameText;
    if (!rich || typeof rich !== "object" || rich.mode !== "rich-context" || !Array.isArray(rich.frames))
      throw new TypeError("semanticFrameText must use rich-context mode with an ordered frame list");
    if (rich.frames.length !== input.frames.length)
      throw new TypeError("semanticFrameText must cover every selected frame exactly once");
    for (let i = 0; i < rich.frames.length; i++) {
      const text = rich.frames[i];
      if (!text || typeof text !== "object" || text.frameIndex !== input.frames[i].index
        || typeof text.head !== "string" || !Array.isArray(text.lines))
        throw new TypeError("semanticFrameText must match selected frame identity and contain only text");
      for (let line = 0; line < text.lines.length; line++) {
        if (typeof text.lines[line] !== "string")
          throw new TypeError("semanticFrameText must match selected frame identity and contain only text");
      }
    }
  }
  for (const warning of input.warnings ?? []) {
    if (warning.kind === "audioInitialGap" && (!Number.isSafeInteger(warning.atMs) || warning.atMs < 0 || warning.atMs >= input.durationMs))
      throw new RangeError("Initial audio gap must precede a captured sample within the recording");
  }
  for (const segment of input.transcript.segments) {
    if (!Number.isSafeInteger(segment.startMs) || segment.startMs < 0 || segment.startMs > input.durationMs)
      throw new RangeError("transcript time must be within the recording duration");
  }
  for (const change of input.contextChanges ?? []) {
    if (!Number.isSafeInteger(change.timeMs) || change.timeMs < 0 || change.timeMs > input.durationMs)
      throw new RangeError("context change time must be within the recording duration");
  }
  for (const event of input.events ?? []) {
    if (!Number.isSafeInteger(event.timeMs) || event.timeMs < 0 || event.timeMs > input.durationMs)
      throw new RangeError("interaction event time must be within the recording duration");
  }
  for (const note of input.notes ?? []) {
    if (!Number.isSafeInteger(note.timeMs) || note.timeMs < 0 || note.timeMs > input.durationMs)
      throw new RangeError("note time must be within the recording duration");
  }
  for (const mark of input.marks ?? []) {
    if (mark.framePath && (!isAbsolute(mark.framePath) || resolve(mark.framePath) !== mark.framePath))
      throw new TypeError("mark frame paths must be absolute and normalized");
  }
  if (input.contactSheetPath && (!isAbsolute(input.contactSheetPath) || resolve(input.contactSheetPath) !== input.contactSheetPath))
    throw new TypeError("contact sheet path must be absolute and normalized");
  if (input.autocrop && (!isAbsolute(input.autocrop.fullFramesDirectory) || resolve(input.autocrop.fullFramesDirectory) !== input.autocrop.fullFramesDirectory))
    throw new TypeError("full frame directory must be absolute and normalized");
}

/**
 * Assemble existing Engine outputs into Record's plain-text handoff format.
 * This function selects no frames and invents no transcript, image, or media facts.
 */
export function assembleProcessingOutput(
  input: ProcessingInput,
  format: "summary" | "local-clipboard" = "summary",
): ProcessingOutput {
  validate(input);
  const clipboard = format === "local-clipboard";
  const richContext = input.frames.length > 0 && input.semanticFrameText !== undefined;
  const lines = [clipboard
    ? `[apparent ${clock(input.durationMs)} · ${input.bundlePath}]`
    : `[apparent ${clock(input.durationMs)} · ${targetLabel(input)} · ${input.bundlePath}]`];
  if (!clipboard && input.destination) {
    const title = input.destination.title || input.destination.session;
    lines.push(`For session: ${title} (${input.destination.session})${input.destination.url ? ` · page ${input.destination.url}` : ""}`);
  }
  if (!clipboard && input.frames.length) {
    const hasClick = (input.events ?? []).some((event) => event.kind === "click" || event.kind === "note") || (input.notes ?? []).length > 0;
    lines.push(hasClick
      ? "Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does."
      : "Visuals are saved in this bundle, but are optional for this text-first handoff. Use apparent zoom if the words need screen context; contact-sheet.png and frames/ are also available there.");
  }
  const context = [
    ...(input.screenAtStart?.trim() ? [`Screen: ${contextValue(input.screenAtStart)}`] : []),
    ...Object.entries(input.context ?? {}).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, value]) => `${contextValue(key)}: ${contextValue(value)}`),
  ];
  if (context.length) lines.push(context.join(" · "));
  if (input.note?.trim()) lines.push(`Note: ${input.note.trim()}`);
  for (const warning of input.warnings ?? []) {
    if (warning.kind === "videoOnlyInteractionTelemetry")
      lines.push("Interaction context: timed CLI video only; clicks and Shift ink were not collected. A visible click in the video will not appear in events, marks or click keyframes. Use the app's recording shortcut for interaction context.");
    else if (warning.kind === "imageQuality") lines.push(`Image quality: ${warning.message}`);
    else if (warning.kind === "noMicrophoneAudio") lines.push("No microphone audio was saved. Check the microphone and try again.");
    else if (warning.kind === "audioInitialGap") lines.push(audioInitialGapText(warning.atMs));
    else if (warning.kind === "noTranscript") lines.push(`No transcript: ${warning.message}`);
    else lines.push("No words transcribed. Your audio was saved—check it or try again closer to the microphone.");
  }
  if (!clipboard && input.autocrop) lines.push(`Cropped to: ${input.autocrop.label} (full frames in ${input.autocrop.fullFramesDirectory}/)`);

  const timeline = richContext ? [] : legacyTimeline(input);
  if (richContext) {
    let shownScreen = input.screenAtStart;
    for (let i = 0; i < input.frames.length; i++) {
      const frame = input.frames[i];
      const screenMarker = frame.screenName && frame.screenName !== shownScreen ? ` ⟨${contextValue(frame.screenName)}⟩` : "";
      shownScreen = frame.screenName ?? shownScreen;
      const flag = frame.markId && input.marks?.some((mark) => mark.id === frame.markId && mark.shape === "flag");
      const flagMarker = flag ? " ⚑" : "";
      const semantic = input.semanticFrameText!.frames[i];
      const head = semantic.head ? ` ${semantic.head}` : "";
      lines.push(`- #${frame.index} (${clock(frame.timeMs)})${screenMarker}${flagMarker}${head}`);
      for (const line of semantic.lines) lines.push(`  ${line}`);
      if (!clipboard) lines.push(`  ${frame.absolutePath}`);
    }
  } else if (input.frames.length) {
    const firstFrameTime = input.frames[0].timeMs;
    for (const item of timeline.filter((entry) => entry.timeMs < firstFrameTime))
      lines.push(`  · ${clock(item.timeMs)} ${item.text}`);
    let shownScreen = input.screenAtStart;
    for (let i = 0; i < input.frames.length; i++) {
      const frame = input.frames[i];
      const screenMarker = frame.screenName && frame.screenName !== shownScreen ? ` ⟨${contextValue(frame.screenName)}⟩` : "";
      shownScreen = frame.screenName ?? shownScreen;
      const flag = frame.markId && input.marks?.some((mark) => mark.id === frame.markId && mark.shape === "flag");
      const flagMarker = flag ? " ⚑" : "";
      // Incomplete annotations use the authoritative timeline instead, keeping
      // speech once without guessing which segment belongs to each frame.
      const said = needsTranscriptTimeline(input) ? undefined : frame.text?.trim();
      if (said) {
        const action = flag ? " ⚑" : frame.reason === "click" ? (() => {
          const compact = (frame.event ?? "").split(" (")[0].trim();
          return compact ? ` [${compact}]` : "";
        })() : "";
        lines.push(`- #${frame.index} (${clock(frame.timeMs)})${screenMarker}${action} “${said}”`);
      } else {
        lines.push(`- #${frame.index} (${clock(frame.timeMs)})${screenMarker}${flagMarker} ${frame.event ?? ""}`.trimEnd());
      }
      if (!clipboard) lines.push(`  ${frame.absolutePath}`);
      const next = input.frames[i + 1]?.timeMs ?? Number.POSITIVE_INFINITY;
      for (const item of timeline.filter((entry) => entry.timeMs >= frame.timeMs && (entry.timeMs < next || i + 1 === input.frames.length)))
        lines.push(`  · ${clock(item.timeMs)} ${item.text}`);
    }
  } else if (timeline.length) {
    for (const item of timeline) lines.push(`· ${clock(item.timeMs)} ${item.text}`);
  } else if (input.transcript.text.trim()) {
    lines.push(`“${input.transcript.text.trim()}”`);
  }
  if (input.videoPath) {
    if (!isAbsolute(input.videoPath) || resolve(input.videoPath) !== input.videoPath)
      throw new TypeError("videoPath must be absolute and normalized");
    if (!clipboard) lines.push(`Video: ${input.videoPath}`);
  }
  if (!clipboard && input.contactSheetPath) lines.push(`Contact sheet: ${input.contactSheetPath}`);
  if (!clipboard) lines.push(`Bundle: ${input.bundlePath}`);
  return { summaryMarkdown: `${lines.join("\n")}\n` };
}


import type { ProcessingInput } from "../index.js";

/** Assembler-authored local metadata. This is not a native Saved or lease receipt. */
export interface CanonicalRecordOutput {
  schema: "apparent.record-output/1";
  transcriptStatus: NonNullable<ProcessingInput["transcriptStatus"]>;
  warnings: Array<{ kind: "audioInitialGap"; atMs: number } | { kind: "transcriptUnavailable" }>;
  /** Relative to this bundle. Read without following links and verify bytes against this seal. */
  acceptedStrokeDocument: { path: string; strokeCount: number; sha256: string; byteLength: number } | null;
}

const object = (value: unknown): value is Record<string, any> => !!value && typeof value === "object" && !Array.isArray(value);
const integer = (value: unknown, max: number) => Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= max;
function reject(): never { throw new TypeError("Invalid canonical Record output metadata"); }
function keys(value: Record<string, any>, allowed: string[]) {
  if (Object.keys(value).some(key => !allowed.includes(key))) reject();
}

/**
 * Read only the optional Record-owned extension in canonical v1 apparent.json.
 * No IO, fallback inference, publication authentication or native availability claim.
 * Legacy absence stays undefined; recognized malformed/unsupported data refuses.
 */
export function readCanonicalRecordOutput(bundle: unknown): CanonicalRecordOutput | undefined {
  if (!object(bundle)) reject();
  if (!Object.hasOwn(bundle, "recordOutput")) return undefined;
  const output = bundle.recordOutput;
  if (bundle.version !== 1 || !integer(bundle.durationMs, Number.MAX_SAFE_INTEGER) || !object(output) || output.schema !== "apparent.record-output/1") reject();
  keys(output, ["schema", "transcriptStatus", "warnings", "acceptedStrokeDocument"]);
  const status = output.transcriptStatus;
  const transcript = bundle.transcript;
  if (!object(status) || !object(transcript) || typeof transcript.text !== "string" || !Array.isArray(transcript.segments)) reject();
  if (status.state === "unavailable") {
    keys(status, ["state"]);
    if (transcript.text.length || transcript.segments.length) reject();
  } else {
    keys(status, ["state", "content", "segmentCount"]);
    if (status.state !== "collected" || !integer(status.segmentCount, 100_000) || status.segmentCount !== transcript.segments.length ||
      transcript.segments.some((s: unknown) => !object(s) || typeof s.text !== "string")) reject();
    const content = transcript.text.trim().length || transcript.segments.some((s: { text: string }) => s.text.trim().length) ? "nonempty" : "empty";
    if (status.content !== content) reject();
  }
  if (!Array.isArray(output.warnings) || output.warnings.length > 2) reject();
  const seen = new Set<string>();
  for (const warning of output.warnings) {
    if (!object(warning) || seen.has(warning.kind)) reject();
    seen.add(warning.kind);
    if (warning.kind === "audioInitialGap") {
      keys(warning, ["kind", "atMs"]);
      if (typeof bundle.audio !== "string" || !integer(warning.atMs, bundle.durationMs - 1)) reject();
    } else if (warning.kind === "transcriptUnavailable") {
      keys(warning, ["kind"]);
      if (status.state !== "unavailable") reject();
    } else reject();
  }
  const companion = output.acceptedStrokeDocument;
  if (!Array.isArray(bundle.strokes)) reject();
  if (companion === null) {
    if (bundle.strokes.length) reject();
  } else {
    if (!object(companion)) reject();
    keys(companion, ["path", "strokeCount", "sha256", "byteLength"]);
    const path = companion.path;
    if (typeof path !== "string" || path.length > 4096 || path.includes("\\") || /[\0\r\n]/u.test(path) || /^[A-Za-z]:/u.test(path) ||
      path.split("/").some((part: string) => !part || part === "." || part === "..") ||
      ["apparent.json", "summary.md", bundle.video, bundle.audio, bundle.contactSheet].includes(path) ||
      (Array.isArray(bundle.keyframes) && bundle.keyframes.some((frame: any) => frame?.file === path)) ||
      !integer(companion.strokeCount, Number.MAX_SAFE_INTEGER) || companion.strokeCount !== bundle.strokes.length ||
      typeof companion.sha256 !== "string" || !/^[0-9a-f]{64}$/u.test(companion.sha256) || !integer(companion.byteLength, Number.MAX_SAFE_INTEGER) || companion.byteLength === 0) reject();
  }
  return structuredClone(output) as CanonicalRecordOutput;
}

import type { Asset, Context } from "./crypto.ts";
import { decodeStrictJson } from "./json.ts";
import { fail } from "./failure.ts";

const ROOT_FILES = new Set(["summary.md", "apparent.json", "notes.json", "contact-sheet.png",
  "contact-sheet-full.png", "recording.mp4", "recording.mov", "audio.wav", "timeline.json", "finish-trace.json"]);
const MEDIA = new Set(["audio.wav", "recording.mp4", "recording.mov"]);
export function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
export function exact(value: Record<string, unknown>, names: string[]): boolean {
  const expected = [...names].sort(), actual = Object.keys(value).sort();
  return actual.length === expected.length && actual.every((key, i) => key === expected[i]);
}
export function assetPath(path: unknown): path is string {
  return typeof path === "string" && path.length <= 128
    && (ROOT_FILES.has(path) || /^(frames|frames-full)\/frame-[0-9]+\.png$/.test(path));
}
export function sameContext(input: unknown, expected: Context): boolean {
  if (!record(input) || !exact(input, ["origin", "accountId", "captureId", "senderGrantId", ...(Object.hasOwn(input, "group") ? ["group"] : [])])) return false;
  return input.origin === expected.origin && input.accountId === expected.accountId
    && input.captureId === expected.captureId && input.senderGrantId === expected.senderGrantId
    && (input.group ?? undefined) === expected.group;
}

export function recipientManifest(input: unknown, expected: Context): Asset[] {
  if (!record(input) || !exact(input, ["version", "context", "files"]) || input.version !== 1) fail("invalid_encryption_archive");
  if (!sameContext(input.context, expected)) fail("encrypted_context_mismatch");
  return recipientAssets(input.files, expected.group);
}
export function recipientAssets(input: unknown, group?: Context["group"]): Asset[] {
  if (!Array.isArray(input) || input.length > 512) fail("invalid_manifest");
  const names = new Set<string>(), files: Asset[] = [];
  let total = 0;
  for (const raw of input) {
    if (!record(raw) || !exact(raw, ["path", "size", "sha256"]) || !assetPath(raw.path)
      || !Number.isSafeInteger(raw.size) || (raw.size as number) < 0 || (raw.size as number) > 90_000_000
      || typeof raw.sha256 !== "string" || !/^[a-f0-9]{64}$/.test(raw.sha256) || names.has(raw.path)) fail("invalid_manifest");
    const file: Asset = { path: raw.path, size: raw.size as number, sha256: raw.sha256 };
    if (file.path === "apparent.json" && file.size > 2_000_000) fail("invalid_manifest");
    names.add(file.path); total += file.size;
    if (total > 500_000_000) fail("quota_exceeded");
    if ((group === "evidence" && MEDIA.has(file.path)) || (group === "media" && !MEDIA.has(file.path))) fail("invalid_manifest");
    files.push(file);
  }
  if (group !== "media" && ["apparent.json", "summary.md", "contact-sheet.png"].some(name => !names.has(name))) fail("invalid_manifest");
  return files;
}

/** Evidence may reference deferred media, but never a missing image or arbitrary path. */
export function validateRecipientReferences(files: Array<Asset & { bytes: Uint8Array }>, context: Context): void {
  if (context.group === "media") return;
  const entry = files.find(f => f.path === "apparent.json");
  if (!entry) fail("unsafe_bundle_reference");
  let value: unknown;
  try { value = decodeStrictJson(entry.bytes, 2_000_000); } catch { fail("unsafe_bundle_reference"); }
  if (!record(value)) fail("unsafe_bundle_reference");
  const names = new Set(files.map(f => f.path));
  const check = (path: unknown, deferred?: Set<string>) => {
    if (path === null) return;
    if (!assetPath(path) || (!names.has(path) && !deferred?.has(path))) fail("unsafe_bundle_reference");
  };
  for (const field of ["contactSheet", "contact_sheet", "video", "audio"]) if (Object.hasOwn(value, field)) {
    const deferred = context.group !== "evidence" ? undefined : field === "audio" ? new Set(["audio.wav"])
      : field === "video" ? new Set(["recording.mp4", "recording.mov"]) : undefined;
    check(value[field], deferred);
  }
  if (value.keyframes !== undefined) {
    if (!Array.isArray(value.keyframes)) fail("unsafe_bundle_reference");
    for (const frame of value.keyframes) {
      if (!record(frame) || !Object.hasOwn(frame, "file")) fail("unsafe_bundle_reference");
      if (context.group === "evidence" && typeof frame.file !== "string") fail("unsafe_bundle_reference");
      check(frame.file);
    }
  }
  if (value.autocrop != null) {
    if (!record(value.autocrop)) fail("unsafe_bundle_reference");
    for (const field of ["fullContactSheet", "full_contact_sheet"]) if (Object.hasOwn(value.autocrop, field)) check(value.autocrop[field]);
    for (const field of ["fullFrames", "full_frames"]) if (Object.hasOwn(value.autocrop, field) && value.autocrop[field] !== "frames-full") fail("unsafe_bundle_reference");
  }
}

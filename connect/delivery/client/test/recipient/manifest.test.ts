import assert from "node:assert/strict";
import { test } from "node:test";
import { recipientManifest } from "../../dist/recipient/manifest.js";
import { RecipientArchiveError } from "../../dist/recipient/archive.js";
import type { Context } from "../../dist/recipient/crypto.js";

const context: Context = { origin: "https://connect.test", accountId: "acct", captureId: "0123456789abcdef0123456789abcdef", senderGrantId: "grant", group: "evidence" };
const hash = "a".repeat(64);
const asset = (path: string, size = 1, sha256 = hash) => ({ path, size, sha256 });
const valid = (files = [asset("apparent.json"), asset("summary.md"), asset("contact-sheet.png")], ctx = context) => ({ version: 1, context: ctx, files });
function rejects(value: unknown, category: string) {
  assert.throws(() => recipientManifest(value, context), (error: unknown) => error instanceof RecipientArchiveError && error.category === category && error.message === category);
}

test("accepts a minimal evidence manifest and returns a detached asset list", () => {
  const input = valid();
  const result = recipientManifest(input, context);
  assert.deepEqual(result, input.files);
  assert.notEqual(result, input.files);
});

test("accepts the media group without evidence-only required references", () => {
  const ctx: Context = { ...context, group: "media" };
  assert.deepEqual(recipientManifest(valid([asset("recording.mp4"), asset("audio.wav")], ctx), ctx).map(f => f.path), ["recording.mp4", "audio.wav"]);
});

test("enforces exact descriptor and context fields", () => {
  rejects({ ...valid(), extra: true }, "invalid_encryption_archive");
  rejects({ ...valid(), version: 2 }, "invalid_encryption_archive");
  rejects({ ...valid(), context: { ...context, extra: true } }, "encrypted_context_mismatch");
  assert.throws(() => recipientManifest(valid(), { ...context, accountId: "other" }), { category: "encrypted_context_mismatch" });
});

test("rejects invalid paths, duplicates, sizes, hashes, and unknown asset fields", () => {
  for (const path of ["../secret", "/tmp/a", "frames/../frame-1.png", "frames/frame-1.jpg", "frames/frame-01.png/extra"]) rejects(valid([asset(path), asset("summary.md"), asset("contact-sheet.png")]), "invalid_manifest");
  rejects(valid([asset("summary.md"), asset("summary.md"), asset("contact-sheet.png")]), "invalid_manifest");
  for (const size of [-1, 90_000_001, 1.5, Number.MAX_SAFE_INTEGER + 1, NaN]) rejects(valid([asset("apparent.json", size), asset("summary.md"), asset("contact-sheet.png")]), "invalid_manifest");
  for (const digest of ["A".repeat(64), "a".repeat(63), "g".repeat(64)]) rejects(valid([asset("apparent.json", 1, digest), asset("summary.md"), asset("contact-sheet.png")]), "invalid_manifest");
  const withExtra = { ...asset("apparent.json"), extra: 1 };
  rejects(valid([withExtra, asset("summary.md"), asset("contact-sheet.png")]), "invalid_manifest");
});

test("enforces group purity and mandatory evidence files", () => {
  rejects(valid([asset("recording.mp4"), asset("apparent.json"), asset("summary.md"), asset("contact-sheet.png")]), "invalid_manifest");
  const media = { ...context, group: "media" as const };
  assert.throws(() => recipientManifest(valid([asset("summary.md")], media), media), { category: "invalid_manifest" });
  for (const required of ["apparent.json", "summary.md", "contact-sheet.png"]) {
    const files = [asset("apparent.json"), asset("summary.md"), asset("contact-sheet.png")].filter(file => file.path !== required);
    rejects(valid(files), "invalid_manifest");
  }
});

test("bounds asset count and aggregate quota without allocating asset contents", () => {
  rejects(valid(Array.from({ length: 513 }, (_, i) => asset(`frames/frame-${i}.png`))), "invalid_manifest");
  const over = [asset("apparent.json", 2_000_000), asset("summary.md", 90_000_000), asset("contact-sheet.png", 90_000_000), ...Array.from({ length: 3 }, (_, i) => asset(`frames/frame-${i}.png`, 90_000_000)), asset("frames/frame-3.png", 48_000_001)];
  rejects(valid(over), "quota_exceeded");
  const boundary = [asset("apparent.json", 2_000_000), asset("summary.md", 90_000_000), asset("contact-sheet.png", 90_000_000), ...Array.from({ length: 3 }, (_, i) => asset(`frames/frame-${i}.png`, 90_000_000)), asset("frames/frame-3.png", 48_000_000)];
  assert.equal(recipientManifest(valid(boundary), context).length, 7);
});

test("bounds bundle metadata before allocating its declared contents", () => {
  const files = [asset("apparent.json", 2_000_000), asset("summary.md"), asset("contact-sheet.png")];
  assert.equal(recipientManifest(valid(files), context)[0]!.size, 2_000_000);
  rejects(valid([{ ...files[0]!, size: 2_000_001 }, ...files.slice(1)]), "invalid_manifest");
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { assembleProcessingOutput, decodeSelectedFrames, type DecodeSelection, type PublishedMediaClient } from "../src/backend/processing/index.ts";

test("assembles semantic text while keeping images and transcript separate", () => {
  const result = assembleProcessingOutput({
    durationMs: 65_000, bundlePath: "/captures/take.apparent",
    transcript: { text: "hello world", segments: [{ startMs: 1_200, text: "hello world" }] },
    frames: [{ index: 2, timeMs: 1_000, absolutePath: "/captures/take.apparent/frames/frame-02.png", text: "Look here" }],
    notes: [{ timeMs: 2_000, text: "Keep this setting" }], videoPath: "/captures/take.apparent/recording.mp4",
  });
  assert.match(result.summaryMarkdown, /\[apparent 1:05 · \/captures\/take\.apparent\]/);
  assert.match(result.summaryMarkdown, /- #2 \(0:01\) “Look here”/);
  assert.match(result.summaryMarkdown, /Read the frame images with this text/);
  assert.match(result.summaryMarkdown, /· 0:01 “hello world”/);
  assert.match(result.summaryMarkdown, /· 0:02 Typed note: Keep this setting/);
  assert.match(result.summaryMarkdown, /Video: \/captures\/take\.apparent\/recording\.mp4/);
  assert.equal((result.summaryMarkdown.match(/hello world/g) ?? []).length, 1);
});

test("public media client receives bundle-clock selection and ordered refs are mapped", async () => {
  const seen: unknown[] = [];
  const media: PublishedMediaClient = { async decode(input) {
    seen.push(input);
    return { jobId: input.jobId, state: "complete", frames: input.requestedBundleMs.map((requestedBundleMs, i) => ({
      requestIndex: i + 1, requestedBundleMs, effectiveVideoMs: Math.max(0, requestedBundleMs - input.source.videoOffsetMs),
      path: `${input.outputDirectory}/frame-${String(i + 1).padStart(3, "0")}.png`, width: 100, height: 80, byteLength: 200, sha256: "a".repeat(64),
    })) };
  } };
  const selection: DecodeSelection = {
    frames: [{ index: 1, timeMs: 100 }, { index: 2, timeMs: 1_250 }], jobId: "b".repeat(32),
    source: { bundleId: "take.apparent", bundlePath: "/captures/take.apparent", videoPath: "/captures/take.apparent/recording.mp4", videoOffsetMs: 250,
      identity: { device: "1", inode: "2", size: 300, modifiedNs: "4" } },
    outputDirectory: "/private/jobs/job/output", limits: { maxFrames: 48, maxEdge: 1280, maxTotalPixels: 64_000_000, deadlineMs: 30_000 },
  };
  const frames = await decodeSelectedFrames(media, selection);
  assert.deepEqual((seen[0] as { requestedBundleMs: number[] }).requestedBundleMs, [100, 1_250]);
  assert.deepEqual(frames.map((frame) => frame.absolutePath), [
    "/private/jobs/job/output/frame-001.png", "/private/jobs/job/output/frame-002.png",
  ]);
  assert.deepEqual(frames.map((frame) => frame.timeMs), [100, 1_250]);
});

test("rejects a partial or reordered decode inventory", async () => {
  const media = { async decode(input: Parameters<PublishedMediaClient["decode"]>[0]) {
    return { jobId: input.jobId, state: "complete" as const, frames: [] };
  } };
  const input: DecodeSelection = {
    frames: [{ index: 1, timeMs: 10 }], jobId: "c".repeat(32),
    source: { bundleId: "take.apparent", bundlePath: "/captures/take.apparent", videoPath: "/captures/take.apparent/recording.mp4", videoOffsetMs: 0,
      identity: { device: "1", inode: "2", size: 3, modifiedNs: "4" } },
    outputDirectory: "/private/job/output", limits: { maxFrames: 1, maxEdge: 100, maxTotalPixels: 10_000, deadlineMs: 1_000 },
  };
  await assert.rejects(decodeSelectedFrames(media, input), /mismatched decode inventory/);
});

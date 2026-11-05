import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { assembleProcessingOutput, decodeSelectedFrames, type DecodeSelection, type PublishedMediaClient } from "../src/backend/processing/index.ts";

test("assembles semantic text while keeping images and transcript separate", () => {
  const bundlePath = fileURLToPath(new URL("./fixtures/take.apparent", import.meta.url));
  const result = assembleProcessingOutput({
    durationMs: 65_000, bundlePath,
    transcript: { text: "hello world", segments: [{ startMs: 1_000, text: "hello world" }] },
    frames: [{ index: 2, timeMs: 1_000, absolutePath: `${bundlePath}/frames/frame-02.png`, text: "hello world" }],
    notes: [{ timeMs: 2_000, text: "Keep this setting" }],
    contactSheetPath: `${bundlePath}/contact-sheet.png`,
  });
  assert.ok(result.summaryMarkdown.includes(`[apparent 1:05 · Display · ${bundlePath}]`));
  assert.match(result.summaryMarkdown, /- #2 \(0:01\) “hello world”/);
  assert.match(result.summaryMarkdown, /Open contact-sheet\.png and frames\//);
  assert.match(result.summaryMarkdown, /· 0:02 Typed note: Keep this setting/);
  assert.ok(result.summaryMarkdown.includes(`  ${bundlePath}/frames/frame-02.png`));
  assert.ok(result.summaryMarkdown.includes(`Contact sheet: ${bundlePath}/contact-sheet.png`));
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

test("untimed transcript survives alongside typed notes", () => {
  const result = assembleProcessingOutput({ durationMs: 5_000, bundlePath: "/captures/notes.apparent",
    transcript: { text: "spoken words", segments: [] }, frames: [],
    notes: [{ timeMs: 2_000, text: "typed context" }] });
  assert.match(result.summaryMarkdown, /“spoken words”/);
  assert.match(result.summaryMarkdown, /· 0:02 Typed note: typed context/);
  assert.equal(result.summaryMarkdown.split("spoken words").length - 1, 1);
});

test("matches the legacy target, session context and screen-change summary fixture", () => {
  const bundlePath = fileURLToPath(new URL("./fixtures/take.apparent", import.meta.url));
  const frame = (index: number) => `${bundlePath}/frames/frame-${String(index).padStart(2, "0")}.png`;
  const longSession = `ses_${"x".repeat(80)}`;
  const summary = assembleProcessingOutput({
    durationMs: 25_000, bundlePath,
    device: { name: "iPhone", app: "OpenCode", appShortVersion: "1.4", appVersion: "354" },
    destination: { session: "ses_demo", title: "Synthetic review", url: "http://127.0.0.1:5173" },
    screenAtStart: "Sessions",
    context: { server: "studio", session: longSession },
    contextChanges: [{ timeMs: 9_000, key: "server", value: "mini" }],
    transcript: { text: "So here. Go back. Done.", segments: [] },
    frames: [
      { index: 1, timeMs: 1_000, absolutePath: frame(1), text: "So here.", event: "scene changed", screenName: "Sessions" },
      { index: 2, timeMs: 3_000, absolutePath: frame(2), event: "tapped “Settings” (x 0.10, y 0.90)", reason: "click", screenName: "Sessions" },
      { index: 3, timeMs: 8_000, absolutePath: frame(3), text: "Go back.", event: "tapped “Back” (x 0.20, y 0.20)", reason: "click", screenName: "Settings › Experimental" },
      { index: 4, timeMs: 20_000, absolutePath: frame(4), text: "Done.", event: "scene changed", screenName: "Settings › Experimental" },
    ],
    events: [{ kind: "click", timeMs: 3_000 }],
    contactSheetPath: `${bundlePath}/contact-sheet.png`,
  });
  const clippedSession = `ses_${"x".repeat(55)}…`;
  assert.equal(summary.summaryMarkdown, [
    `[apparent 0:25 · iPhone · OpenCode 1.4 (354) · ${bundlePath}]`,
    "For session: Synthetic review (ses_demo) · page http://127.0.0.1:5173",
    "Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does.",
    `Screen: Sessions · server: studio · session: ${clippedSession}`,
    `- #1 (0:01) “So here.”`, `  ${frame(1)}`,
    `- #2 (0:03) tapped “Settings” (x 0.10, y 0.90)`, `  ${frame(2)}`,
    `- #3 (0:08) ⟨Settings › Experimental⟩ [tapped “Back”] “Go back.”`, `  ${frame(3)}`,
    "  · 0:09 server → mini",
    `- #4 (0:20) “Done.”`, `  ${frame(4)}`,
    `Contact sheet: ${bundlePath}/contact-sheet.png`,
    `Bundle: ${bundlePath}`,
    "",
  ].join("\n"));
});

test("matches legacy mixed spoken, typed note, mark pin, warning and crop semantics", () => {
  const bundlePath = fileURLToPath(new URL("./fixtures/take.apparent", import.meta.url));
  const firstFrame = `${bundlePath}/frames/frame-01.png`;
  const secondFrame = `${bundlePath}/frames/frame-02.png`;
  const summary = assembleProcessingOutput({
    durationMs: 14_000, bundlePath, target: { app: "App", windowTitle: "Window" }, note: "Simplify it.",
    transcript: { text: "This row is too dense. Later spoken feedback", segments: [
      { startMs: 3_000, text: "This row is too dense." }, { startMs: 9_000, text: "Later spoken feedback" },
    ] },
    frames: [
      { index: 1, timeMs: 5_100, absolutePath: firstFrame, text: "This row is too dense.", event: "tapped x 0.50, y 0.50", markId: "m1" },
      { index: 2, timeMs: 9_000, absolutePath: secondFrame, text: "Later spoken feedback", event: "scene changed" },
    ],
    marks: [{ id: "m1", shape: "tap", pin: 1, framePath: firstFrame }],
    events: [{ kind: "note", timeMs: 6_000, pin: 1, text: "Exact typed feedback\nSecond line" }],
    warnings: [{ kind: "imageQuality", message: "Some frames use lower-resolution candidates." }],
    autocrop: { label: "App", fullFramesDirectory: `${bundlePath}/frames` },
  });
  assert.equal(summary.summaryMarkdown, [
    `[apparent 0:14 · App · ${bundlePath}]`,
    "Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does.",
    "Note: Simplify it.",
    "Image quality: Some frames use lower-resolution candidates.",
    `Cropped to: App (full frames in ${bundlePath}/frames/)`,
    "- #1 (0:05) “This row is too dense.”", `  ${firstFrame}`,
    `  · 0:06 Typed note · pin 1 · ${firstFrame}:`,
    "    Exact typed feedback", "    Second line",
    "- #2 (0:09) “Later spoken feedback”", `  ${secondFrame}`,
    `Bundle: ${bundlePath}`,
    "",
  ].join("\n"));
});

test("processing warnings preserve explicit video-only and transcript failure facts", () => {
  const output = assembleProcessingOutput({ durationMs: 3_000, bundlePath: "/captures/warnings.apparent",
    transcript: { text: "", segments: [] }, frames: [],
    warnings: [{ kind: "videoOnlyInteractionTelemetry" }, { kind: "noTranscript", message: "The selected model was unavailable." }] });
  assert.match(output.summaryMarkdown, /timed CLI video only; clicks and Shift ink were not collected/);
  assert.match(output.summaryMarkdown, /No transcript: The selected model was unavailable\./);
});


test("window titles remain outside semantic handoff text", () => {
  const output = assembleProcessingOutput({ durationMs: 3_000, bundlePath: "/captures/private.apparent",
    target: { app: "Browser", windowTitle: "Private document title" },
    transcript: { text: "", segments: [] }, frames: [] });
  assert.match(output.summaryMarkdown, /· Browser ·/);
  assert.doesNotMatch(output.summaryMarkdown, /Private document title/);
});

for (const annotated of [false, true]) test(`incomplete frame speech annotations preserve the transcript (partial=${annotated})`, () => {
  const output = assembleProcessingOutput({ durationMs: 4_000, bundlePath: "/captures/speech.apparent",
    transcript: { text: "First words. Later words.", segments: [
      { startMs: 500, text: "First words." }, { startMs: 2_000, text: "Later words." },
    ] },
    frames: [{ index: 1, timeMs: 1_000, absolutePath: "/captures/speech.apparent/frame.png", text: annotated ? "First words." : undefined }],
    notes: [{ timeMs: 3_000, text: "Typed context" }] });
  assert.match(output.summaryMarkdown, /First words\./);
  assert.match(output.summaryMarkdown, /· 0:02 “Later words\.”/);
  assert.match(output.summaryMarkdown, /Typed context/);
});

test("untimed speech survives selected frames without complete annotations", () => {
  const output = assembleProcessingOutput({ durationMs: 3_000, bundlePath: "/captures/speech.apparent",
    transcript: { text: "Unsegmented words", segments: [] },
    frames: [{ index: 1, timeMs: 1_000, absolutePath: "/captures/speech.apparent/frame.png" }] });
  assert.match(output.summaryMarkdown, /Unsegmented words/);
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { formatLocalHandoff, type LocalHandoffInput } from "../../dist/index.js";

const base: LocalHandoffInput = {
  savedBundle: { status: "saved", bundlePath: "/captures/a user's take.apparent" },
  processing: {
    durationMs: 65_000,
    bundlePath: "/captures/a user's take.apparent",
    transcript: { text: "Please change this control", segments: [
      { startMs: 1_200, text: "Please change this control" },
      { startMs: 3_000, text: "Keep the warning visible" },
    ] },
    target: { app: "Editor", windowTitle: "PRIVATE WINDOW TITLE" },
    destination: { session: "private-session", title: "PRIVATE TOKEN abc123", url: "https://private.test/?token=abc123" },
    screenAtStart: "Editor",
    context: { project: "Synthetic project" },
    contextChanges: [{ timeMs: 2_000, key: "selection", value: "Save button" }],
    frames: [{ index: 1, timeMs: 1_000, absolutePath: "/captures/a user's take.apparent/frames/frame-01.png", text: "Please change this control" }],
    notes: [{ timeMs: 4_000, text: "Keep this note" }],
    warnings: [{ kind: "imageQuality", message: "Image quality is reduced." }],
    autocrop: { label: "the activity", fullFramesDirectory: "/captures/a user's take.apparent/frames-full" },
    videoPath: "/captures/a user's take.apparent/recording.mp4",
  },
  requestVideoProof: false,
};

test("formats the legacy local handoff order and quotes paths safely", () => {
  const { text } = formatLocalHandoff(base);
  assert.match(text, /^apparent inspect '\/captures\/a user'\\''s take\.apparent'\n\nOnce per agent session/);
  assert.ok(text.indexOf("apparent inspect") < text.indexOf("Once per agent session"));
  assert.ok(text.indexOf("Once per agent session") < text.indexOf("Inspect contact-sheet.png"));
  assert.ok(text.indexOf("Inspect contact-sheet.png") < text.indexOf("[apparent 1:05"));
  assert.ok(text.indexOf("[apparent 1:05") < text.indexOf("Image quality: Image quality is reduced."));
  assert.match(text, /Uncropped images are in \/captures\/a user's take\.apparent\/frames-full\//);
  assert.match(text, /· 0:01 “Please change this control”/);
  assert.match(text, /· 0:04 Typed note: Keep this note/);
  assert.doesNotMatch(text, /Video:|Bundle:|Cropped to:|Open contact-sheet/);
  assert.ok(text.endsWith("  · 0:04 Typed note: Keep this note\n"));
  assert.match(text, /Screen: Editor · project: Synthetic project/);
  assert.match(text, /selection → Save button/);
  assert.equal(text.split("Please change this control").length - 1, 1);
  assert.ok(!text.includes("make-it-apparent"));
  assert.ok(!text.includes("PRIVATE WINDOW TITLE") && !text.includes("PRIVATE TOKEN") && !text.includes("abc123") && !text.includes("xyz789"));
});

test("local handoff uses rich processing chunks without re-adding the legacy timeline", () => {
  const bundlePath = "/synthetic/rich-handoff.apparent";
  const { text } = formatLocalHandoff({
    savedBundle: { status: "saved", bundlePath },
    requestVideoProof: false,
    processing: {
      durationMs: 5_000,
      bundlePath,
      target: { app: "Editor", windowTitle: "PRIVATE WINDOW TITLE" },
      screenAtStart: "Screen One",
      transcript: { text: "Legacy whole-transcript rendering must be suppressed.", segments: [
        { startMs: 500, text: "Legacy whole-transcript rendering must be suppressed." },
      ] },
      frames: [{ index: 1, timeMs: 500, absolutePath: `${bundlePath}/frames/1.png` }],
      events: [{ kind: "note", timeMs: 1_000, pin: 1, text: "legacy note duplicate" }],
      notes: [{ timeMs: 1_000, text: "legacy note duplicate", pin: 1 }],
      contextChanges: [{ timeMs: 2_000, key: "legacy context duplicate", value: "hidden" }],
      warnings: [{ kind: "imageQuality", message: "Keep this warning." }],
      autocrop: { label: "Editor", fullFramesDirectory: `${bundlePath}/frames-full` },
      semanticFrameText: { mode: "rich-context", frames: [{
        frameIndex: 1,
        head: "“Open the menu,”",
        lines: ["✎① 0:00 pin 1", "· 0:01 Typed note · pin 1 · frames/1.png:\n    Keep this\n    for review", "· 0:02 tab → Preview"],
      }] },
    },
  });
  assert.match(text, /Inspect contact-sheet\.png and relevant frames/);
  assert.match(text, /Screen: Screen One/);
  assert.match(text, /Image quality: Keep this warning\./);
  assert.match(text, /Uncropped images are in \/synthetic\/rich-handoff\.apparent\/frames-full\//);
  assert.match(text, /- #1 \(0:00\) “Open the menu,”\n  ✎① 0:00 pin 1/);
  assert.match(text, /Typed note · pin 1 · frames\/1\.png:\n    Keep this\n    for review/);
  assert.match(text, /· 0:02 tab → Preview/);
  assert.doesNotMatch(text, /Legacy whole-transcript|legacy note duplicate|legacy context duplicate|PRIVATE WINDOW TITLE/);
});

test("preserves the supplied video-proof opt-out exactly", () => {
  assert.ok(formatLocalHandoff({ ...base, requestVideoProof: false }).text.includes("Once per agent session"));
  assert.ok(!formatLocalHandoff({ ...base, requestVideoProof: false }).text.includes("make-it-apparent"));
  assert.ok(formatLocalHandoff({ ...base, requestVideoProof: true }).text.includes(
    "Use make-it-apparent for complex visible changes; share verified video proof or explain the blocker.",
  ));
});

test("requires explicit proof policy and accepts only normalized crop paths", () => {
  assert.throws(() => formatLocalHandoff({ ...base, requestVideoProof: undefined as never }), /supplied explicitly/);
  assert.throws(() => formatLocalHandoff({ ...base, processing: { ...base.processing, autocrop: { label: "crop", fullFramesDirectory: "frames-full" } } }), /absolute and normalized/);
});

test("requires a caller-supplied saved bundle fact matching semantic input", () => {
  assert.throws(() => formatLocalHandoff({ ...base, savedBundle: { status: "saved", bundlePath: "/other/take.apparent" } }), /identify the processed bundle/);
});

test("video-only handoff states missing facts without guessing GUI or CLI origin", () => {
  const input = structuredClone(base);
  input.processing.warnings = [{ kind: "videoOnlyInteractionTelemetry" }];
  const text = formatLocalHandoff(input).text;
  assert.match(text, /Interaction context: video only; clicks and Shift ink were not collected\. A visible click in the video will not appear in events, marks or click keyframes\./);
  assert.doesNotMatch(text, /timed CLI|recording shortcut/);
  assert.deepEqual(input.processing.warnings, [{ kind: "videoOnlyInteractionTelemetry" }]);
});

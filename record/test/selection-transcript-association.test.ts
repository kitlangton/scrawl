import assert from "node:assert/strict";
import { test } from "node:test";
import { associateTranscriptWithSelectedFrames, type SelectedFrameReference } from "../src/backend/selection/transcript-association.ts";

const frames: SelectedFrameReference[] = [
  { id: "frame-1", timeMs: 1_000, absolutePath: "/capture/frames/frame-01.png" },
  { id: "frame-2", timeMs: 4_000, absolutePath: "/capture/frames/frame-02.png" },
];

test("partitions timed words across selected frames without losing punctuation", () => {
  const output = associateTranscriptWithSelectedFrames(frames, {
    text: "Make this smaller, and move that.",
    words: [
      [0, "Make"], [1_000, "this"], [2_000, "smaller"], [2_100, ","],
      [3_000, "and"], [4_000, "move"], [7_000, "that."],
    ].map(([startMs, text]) => ({ startMs: Number(startMs), endMs: Number(startMs) + 100, text: String(text) })),
    segments: [{ startMs: 0, endMs: 7_100, text: "Make this smaller, and move that." }],
  });
  assert.deepEqual(output.map(({ said }) => said), ["Make this smaller,", "and move that."]);
  assert.deepEqual(output.map(({ absolutePath }) => absolutePath), frames.map(({ absolutePath }) => absolutePath));
});

test("segment-only transcript follows clause boundaries and does not create frames", () => {
  const output = associateTranscriptWithSelectedFrames(frames, {
    segments: [
      { startMs: 0, endMs: 2_000, text: "First thought." },
      { startMs: 4_000, endMs: 6_000, text: "Second thought." },
    ],
  });
  assert.deepEqual(output.map(({ said }) => said), ["First thought.", "Second thought."]);
  assert.equal(output.length, frames.length);
});

test("action anchor and punctuation at the next frame boundary preserve clause ownership", () => {
  const output = associateTranscriptWithSelectedFrames([
    { id: "first", timeMs: 1_000, anchorTimeMs: 900, absolutePath: "/capture/frames/one.png" },
    { id: "second", timeMs: 5_000, anchorTimeMs: 5_000, absolutePath: "/capture/frames/two.png" },
  ], {
    words: [[0, "Lots"], [500, "of"], [900, "glitches"], [5_000, "."], [5_200, "Yeah"]]
      .map(([startMs, text]) => ({ startMs: Number(startMs), endMs: Number(startMs) + 100, text: String(text) })),
    segments: [],
  });
  assert.deepEqual(output.map(({ said }) => said), ["Lots of glitches.", "Yeah"]);
});

test("rejects unavailable identity evidence: duplicate IDs, unordered times, and stale paths", () => {
  assert.throws(() => associateTranscriptWithSelectedFrames([
    frames[0]!, { ...frames[1]!, id: frames[0]!.id },
  ], { segments: [] }), /unique/);
  assert.throws(() => associateTranscriptWithSelectedFrames([
    frames[1]!, frames[0]!,
  ], { segments: [] }), /timestamp order/);
  assert.throws(() => associateTranscriptWithSelectedFrames([
    { ...frames[0]!, absolutePath: "/capture/../private/frame.png" },
  ], { segments: [] }), /absolute native path/);
  assert.throws(() => associateTranscriptWithSelectedFrames(frames, {
    segments: [{ startMs: 2, endMs: 1, text: "bad timing" }],
  }), /timestamps/);
});

test("untimed transcript is attached once to the first selected frame", () => {
  const output = associateTranscriptWithSelectedFrames(frames, {
    text: "spoken words without timing", segments: [],
  });
  assert.deepEqual(output.map(({ said }) => said), ["spoken words without timing", undefined]);
});

test("zero-time tolerance and punctuation tokenization match the Rust source", () => {
  const selected = [40, 50].map((timeMs, index) => ({ id: `f-${index}`, timeMs, absolutePath: "/frame.png" }));
  assert.deepEqual(associateTranscriptWithSelectedFrames(selected, {
    words: [{ startMs: 0, endMs: 0, text: "word" }], segments: [],
  }).map(frame => frame.said), [undefined, "word"]);
});

for (const [text, expected] of [
  ["one\u0085two", ["one\u0085two,", "next"]],
  ["one\ufefftwo", ["one\ufefftwo, next", undefined]],
  ["x \u0345", ["x \u0345,", "next"]],
  ["hello,world", ["hello,world, next", undefined]],
] as const) test(`Rust Unicode word boundaries: ${JSON.stringify(text)}`, () => {
  const output = associateTranscriptWithSelectedFrames([
    { id: "f-1", timeMs: 0, absolutePath: "/one.png" },
    { id: "f-2", timeMs: 280, absolutePath: "/two.png" },
  ], { segments: [], words: [
    { startMs: 0, endMs: 100, text },
    { startMs: 100, endMs: 200, text: "," },
    { startMs: 200, endMs: 300, text: "next" },
  ] });
  assert.deepEqual(output.map(frame => frame.said), expected);
});

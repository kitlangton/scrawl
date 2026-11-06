import assert from "node:assert/strict";
import test from "node:test";
import { selectVisualKeyframes, visualKeyframeBudget, type VisualSelectionSample } from "../../src/backend/selection/visual-keyframes.ts";

const WIDTH = 64;
const HEIGHT = 36;
function sample(id: string, timeMs: number, value = 0, width = 1280, height = 720): VisualSelectionSample {
  return { id, timeMs, width, height, thumbnail: Array(WIDTH * HEIGHT).fill(value) };
}
function blockSample(id: string, timeMs: number, blocks: number, value = 1): VisualSelectionSample {
  const thumbnail = Array(WIDTH * HEIGHT).fill(0);
  for (let block = 0; block < blocks; block++) {
    const blockX = block % 8;
    const blockY = Math.floor(block / 8);
    for (let y = blockY * 6; y < blockY * 6 + 6; y++)
      for (let x = blockX * 8; x < blockX * 8 + 8; x++) thumbnail[y * WIDTH + x] = value;
  }
  return { id, timeMs, width: 1280, height: 720, thumbnail };
}

test("static visual input selects the donor midpoint and explicit zero stays empty", () => {
  const selected = selectVisualKeyframes([sample("a", 0), sample("b", 1_000), sample("c", 2_000)], { durationMs: 2_000 });
  assert.deepEqual(selected, [{ sampleId: "b", timeMs: 1_000, span: { startMs: 1_000, endMs: 1_000 }, reason: "initial", score: 0, event: "initial view" }]);
  assert.deepEqual(selectVisualKeyframes([sample("a", 0)], { durationMs: 0, maxKeyframes: 0 }), []);
  assert.deepEqual(selectVisualKeyframes([], { durationMs: 0 }), []);
});

test("changing runs collapse to the last changed sample and retain their complete time span", () => {
  const selected = selectVisualKeyframes([sample("a", 0), sample("b", 100, .2), sample("c", 200, .4), sample("d", 300, .4)], { durationMs: 300 });
  assert.deepEqual(selected, [{ sampleId: "c", timeMs: 200, span: { startMs: 0, endMs: 200 }, reason: "scene_change", score: 51.19999694824219, event: "scene changed" }]);
});

test("local 8×6 block evidence detects four changed blocks while one small block falls back", () => {
  const one = selectVisualKeyframes([sample("before", 0), blockSample("one-block", 100, 1)], { durationMs: 100 });
  assert.equal(one[0]?.reason, "initial");
  const four = selectVisualKeyframes([sample("before", 0), blockSample("four-blocks", 100, 4)], { durationMs: 100 });
  assert.equal(four[0]?.sampleId, "four-blocks");
  assert.equal(four[0]?.score, 80);
});

test("f32 scene threshold and score rounding match the Engine donor", () => {
  const atThreshold = Math.fround(Math.fround(.08) / Math.fround(1.4));
  const bytes = new ArrayBuffer(4);
  const view = new DataView(bytes);
  view.setFloat32(0, atThreshold);
  view.setUint32(0, view.getUint32(0) - 1);
  const below = view.getFloat32(0);
  assert.equal(selectVisualKeyframes([sample("a", 0), blockSample("b", 100, 4, below)], { durationMs: 100 })[0]?.reason, "initial");
  const selected = selectVisualKeyframes([sample("a", 0), blockSample("b", 100, 4, atThreshold)], { durationMs: 100 });
  assert.equal(selected[0]?.reason, "scene_change");
  assert.equal(selected[0]?.score, 43.20000076293945);
});

test("score ties rank by time, spacing rejects under 750 ms and admits exactly 750 ms", () => {
  const under = selectVisualKeyframes([sample("a", 0), sample("b", 100, .2), sample("b-hold", 200, .2), sample("c", 800, .4), sample("c-hold", 900, .4)], { durationMs: 900, maxKeyframes: 2 });
  assert.deepEqual(under.map(({ sampleId }) => sampleId), ["b"]);
  const edge = selectVisualKeyframes([sample("a", 0), sample("b", 100, .2), sample("b-hold", 200, .2), sample("c", 850, .4), sample("c-hold", 950, .4)], { durationMs: 950, maxKeyframes: 2 });
  assert.deepEqual(edge.map(({ sampleId }) => sampleId), ["b", "c"]);

  const tied = selectVisualKeyframes([sample("a", 0), sample("early", 100, .2), sample("hold-early", 200, .2), sample("later", 1_000, .4), sample("hold-later", 1_100, .4)], { durationMs: 1_100, maxKeyframes: 1 });
  assert.deepEqual(tied.map(({ sampleId }) => sampleId), ["early"]);
});

test("novelty rejects repeated settled screens and coverage adds ranked visual splits", () => {
  const repeated = selectVisualKeyframes([
    sample("a0", 0), sample("b1", 100, .3), sample("b2", 200, .3),
    sample("a1", 1_000), sample("a2", 1_100), sample("b3", 2_000, .3), sample("b4", 2_100, .3),
  ], { durationMs: 2_100, maxKeyframes: 6 });
  assert.deepEqual(repeated.map(({ sampleId }) => sampleId), ["b1", "a1"]);

  const covered = selectVisualKeyframes([
    sample("a0", 0), sample("b1", 10_000, .2), sample("b-hold", 11_000, .2),
    sample("c1", 40_000, .4), sample("c-hold", 41_000, .4), sample("d1", 80_000, .6), sample("d-hold", 81_000, .6),
  ], { durationMs: 81_000, maxKeyframes: 7 });
  assert.deepEqual(covered.map(({ sampleId }) => sampleId), ["b1", "c1", "d1"]);
});

test("duration budget matches donor boundaries and cap", () => {
  assert.deepEqual([0, 19_999, 20_000, 39_999, 40_000, 67_600, 140_000, 268_000].map(visualKeyframeBudget), [6, 6, 6, 6, 7, 8, 12, 12]);
});

test("malformed sample identity, timing, dimensions and measurements fail closed", () => {
  const base = [sample("a", 0), sample("b", 100, .2)];
  assert.throws(() => selectVisualKeyframes([sample("", 0)], { durationMs: 100 }), /IDs/);
  assert.throws(() => selectVisualKeyframes([sample("a", 0), sample("a", 100)], { durationMs: 100 }), /IDs/);
  assert.throws(() => selectVisualKeyframes([sample("b", 100), sample("a", 0)], { durationMs: 100 }), /times/);
  assert.throws(() => selectVisualKeyframes([sample("a", 101)], { durationMs: 100 }), /times/);
  assert.throws(() => selectVisualKeyframes([sample("a", 0, 0, 0, 720)], { durationMs: 100 }), /dimensions/);
  assert.throws(() => selectVisualKeyframes([sample("a", 0, 0, 32_769, 720)], { durationMs: 100 }), /bounded/);
  assert.throws(() => selectVisualKeyframes([sample("a", 0), sample("b", 100, .2, 640, 480)], { durationMs: 100 }), /same decoded plane/);
  assert.throws(() => selectVisualKeyframes([{ ...base[0]!, thumbnail: base[0]!.thumbnail.slice(1) }], { durationMs: 100 }), /exactly 2304/);
  assert.throws(() => selectVisualKeyframes([{ ...base[0]!, thumbnail: base[0]!.thumbnail.map((_, index) => index === 4 ? Number.NaN : 0) }], { durationMs: 100 }), /finite/);
  assert.throws(() => selectVisualKeyframes([{ ...base[0]!, thumbnail: base[0]!.thumbnail.map((_, index) => index === 4 ? 1.01 : 0) }], { durationMs: 100 }), /\[0, 1\]/);
  assert.throws(() => selectVisualKeyframes(Array.from({ length: 481 }, (_, index) => sample(`s${index}`, index)), { durationMs: 500 }), /at most 480/);
  assert.throws(() => selectVisualKeyframes(base, { durationMs: Number.POSITIVE_INFINITY }), /durationMs/);
  assert.throws(() => selectVisualKeyframes(base, { durationMs: 100, maxKeyframes: -1 }), /maxKeyframes/);
  assert.throws(() => selectVisualKeyframes(base, { durationMs: 100, maxKeyframes: 13 }), /maxKeyframes/);
});

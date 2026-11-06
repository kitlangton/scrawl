import test from "node:test";
import assert from "node:assert/strict";
import { crc32, deflateSync } from "node:zlib";
import { readFrameVisualObservations } from "../src/observations/index.ts";
import { prepareMedia2Request } from "../src/media2/index.ts";
import { fixture, seal } from "./media2/fixture.ts";

function chunk(type: string, body: Buffer) {
  const result = Buffer.alloc(body.length + 12);
  result.writeUInt32BE(body.length); result.write(type, 4); body.copy(result, 8);
  result.writeUInt32BE(crc32(result.subarray(4, -4)), result.length - 4);
  return result;
}
function profile(width = 100, height = 50) {
  const columns = Math.min(width, 128), rows = Math.min(height, 128);
  const p = Buffer.alloc(20 + (64 * 36 + columns * rows) * 4);
  p.write("APOBS001"); p.writeUInt32LE(width, 8); p.writeUInt32LE(height, 12);
  p.writeUInt16LE(columns, 16); p.writeUInt16LE(rows, 18);
  for (let at = 20; at < p.length; at += 4) p.writeFloatLE(0.25, at);
  return p;
}
function png(profiles: Buffer[] = [profile()], width = 100, height = 50, extra: Buffer[] = []) {
  const header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(Buffer.alloc((width * 4 + 1) * height))), ...profiles.map(p => chunk("apOB", p)), ...extra, chunk("IEND", Buffer.alloc(0))]);
}
function input(content = png(), width = 100, height = 50) {
  const f = fixture(); const result = structuredClone(f.result);
  f.request.limits.maxEdge = Math.max(100, width, height);
  f.request.limits.maxTotalPixels = 1_000_000;
  result.requestSha256 = prepareMedia2Request(f.request, f.materials).sha256;
  result.frames[0].width = width; result.frames[0].height = height;
  result.frames[0].file = seal("frames/1.png", content); result.frames[0].byteLength = content.length;
  return { document: f.document(result), materials: f.materials, requestIndex: 1, pngBytes: content, maxFileBytes: content.length };
}

test("observations bind ordered selector, material clock, lease, original request and full sealed PNG", () => {
  const i = input(); const result = readFrameVisualObservations(i);
  assert.equal(result.state, "measured"); if (result.state !== "measured") return;
  assert.deepEqual(result.pixels, { width: 100, height: 50, origin: "decoded-derivative-rgba8", alpha: "ignored" });
  assert.equal(result.thumbnail.cells.length, 2304); assert.equal(result.regions.cells.length, 5000);
  assert.equal(result.thumbnail.cells[0], 0.25); assert.equal(result.source.requestIndex, 1);
  assert.deepEqual(result.source.materials, i.materials.manifest.key);
  assert.equal(result.source.leaseId, i.materials.manifest.leaseId);
  assert.deepEqual(result.source.clock, i.materials.payload.clock);
  assert.equal(result.source.collection.candidates, "unavailable");
  assert.equal(result.source.file.sha256, i.document.envelope && (i.document.envelope as { payload: { frames: { file: { sha256: string } }[] } }).payload.frames[0].file.sha256);
  assert.throws(() => readFrameVisualObservations({ ...i, requestIndex: 2 }), /length|SHA256/);
  assert.throws(() => readFrameVisualObservations({ ...i, requestIndex: 0 }), /requestIndex/);
  assert.throws(() => readFrameVisualObservations({ ...i, document: { ...i.document, request: { ...i.document.request, leaseId: "d".repeat(32) } } }), /lease/);
});

test("old PNGs and recognized future profiles are explicitly unavailable", () => {
  assert.equal(readFrameVisualObservations(input(png([]))).state, "unavailable");
  const p = profile(); p.write("APOBS002");
  const r = readFrameVisualObservations(input(png([p])));
  assert.equal(r.state, "unavailable"); if (r.state === "unavailable") assert.equal(r.reason, "observation-profile-unsupported");
  const malformed = profile(); malformed.write("notmagic");
  assert.throws(() => readFrameVisualObservations(input(png([malformed]))), /identifier/);
  const nonAscii = profile(); nonAscii[0] |= 128;
  assert.throws(() => readFrameVisualObservations(input(png([nonAscii]))), /identifier/);
});

test("small, odd and large derivative planes retain exact min-128 region shape", () => {
  for (const [width, height] of [[1, 1], [3, 5], [257, 129]]) {
    const r = readFrameVisualObservations(input(png([profile(width, height)], width, height), width, height));
    assert.equal(r.state, "measured"); if (r.state !== "measured") continue;
    assert.equal(r.regions.columns, Math.min(width, 128));
    assert.equal(r.regions.rows, Math.min(height, 128));
    assert.equal(r.regions.cells.length, Math.min(width, 128) * Math.min(height, 128));
  }
});

test("observations reject duplicate profiles, extra payload, CRC damage and file mutation", () => {
  for (const profiles of [[profile(), profile()], [Buffer.concat([profile(), Buffer.alloc(4)])], [profile().subarray(0, -4)]]) {
    assert.throws(() => readFrameVisualObservations(input(png(profiles))), /duplicate|length/);
  }
  const damaged = png(); damaged[damaged.length - 5] ^= 1;
  assert.throws(() => readFrameVisualObservations(input(damaged)), /CRC/);
  const i = input(); i.pngBytes[50] ^= 1;
  assert.throws(() => readFrameVisualObservations(i), /SHA256/);
  assert.throws(() => readFrameVisualObservations({ ...input(), maxFileBytes: 1 }), /budget/);
  const original = png([]);
  const beforeImageData = Buffer.concat([original.subarray(0, 33), chunk("apOB", profile()), original.subarray(33)]);
  assert.throws(() => readFrameVisualObservations(input(beforeImageData)), /after image data/);
});

test("PNG, profile and receipt dimensions must agree with exact bounded region shape", () => {
  assert.throws(() => readFrameVisualObservations(input(png([profile(99, 50)]))), /dimensions/);
  assert.throws(() => readFrameVisualObservations(input(png([profile()], 99, 50))), /IHDR/);
  const p = profile(); p.writeUInt16LE(99, 16);
  assert.throws(() => readFrameVisualObservations(input(png([p]))), /region dimensions/);
  const header = Buffer.alloc(13); header.writeUInt32BE(100); header.writeUInt32BE(50, 4); header[8] = 8; header[9] = 2;
  const content = png(); chunk("IHDR", header).copy(content, 8);
  assert.throws(() => readFrameVisualObservations(input(content)), /RGBA8/);
});

test("all native float cells are finite unit-range float32, without numeric rewriting", () => {
  for (const value of [NaN, Infinity, -Infinity, -0.001, 1.001]) {
    for (const at of [20, 20 + 2304 * 4]) {
      const p = profile(); p.writeFloatLE(value, at);
      assert.throws(() => readFrameVisualObservations(input(png([p]))), /finite unit/);
    }
  }
  const p = profile(); p.writeFloatLE(1 / 3, 20);
  const r = readFrameVisualObservations(input(png([p])));
  assert.equal(r.state === "measured" && r.thumbnail.cells[0], Math.fround(1 / 3));
});

test("PNG walking rejects truncation, trailing data, unknown critical chunks and chunk floods", () => {
  for (const content of [png().subarray(0, -1), Buffer.concat([png(), Buffer.from([0])]), png([], 100, 50, [chunk("BOOM", Buffer.alloc(0))])]) {
    assert.throws(() => readFrameVisualObservations(input(content)), /PNG/);
  }
  const flood = png([], 100, 50, Array.from({ length: 16_384 }, () => chunk("teST", Buffer.alloc(0))));
  assert.throws(() => readFrameVisualObservations(input(flood)), /bounds/);
  const huge = png(); huge.writeUInt32BE(0xffffffff, 8);
  assert.throws(() => readFrameVisualObservations(input(huge)), /oversized/);
  assert.throws(() => readFrameVisualObservations(input(png([], 100, 50, [chunk("PLTE", Buffer.alloc(3))]))), /palette/);
  assert.throws(() => readFrameVisualObservations(input(png([], 100, 50, [chunk("teST", Buffer.alloc(0)), chunk("IDAT", Buffer.from([0]))]))), /nonconsecutive/);
  assert.throws(() => readFrameVisualObservations(input(png([Buffer.alloc(74_773)]))), /bounds/);
});

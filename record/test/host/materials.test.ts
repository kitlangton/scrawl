import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmod, link, lstat, mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import type { MaterialsManifest } from "@apparent/engine-sdk/capture";
import { readCaptureMaterials, type CaptureMaterialsExpected, type CaptureSealedFile } from "../../dist/backend/host/materials.js";

const enc = (x: unknown) => new TextEncoder().encode(JSON.stringify(x));
const sha = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");
const identity = { captureId: "a".repeat(32), materialsId: "b".repeat(32) };
const expected: CaptureMaterialsExpected = { key: { ...identity, revision: "0".repeat(64) }, leaseId: "c".repeat(32), bundleId: "materials-test" };

async function fixture(factIdentity = identity, safeForProcessing = true, includeStroke = false) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-material-reader-"))); await chmod(root, 0o700);
  await mkdir(join(root, "facts"), { mode: 0o700 });
  const source = { id: "display", generation: "d".repeat(32), confirmedAtMs: 0, kind: "display" as const, displayId: 1,
    rectangle: { x: 0, y: 0, width: 10, height: 10, scale: 1 }, confirmation: { kind: "explicit-display" as const, id: "e".repeat(32) } };
  const invocation = { kind: "explicit" as const, invocationId: "f".repeat(32), observedAtMs: 0 };
  const ctx = enc({ schema: "apparent.capture-context/1", identity: factIdentity, source, invocation,
    context: { state: "unavailable", reason: "not collected" }, session: { state: "unavailable", reason: "not collected" },
    windows: { state: "unavailable", reason: "not collected" }, cursor: { state: "unavailable", reason: "not collected" } });
  await writeFile(join(root, "facts/context.json"), ctx, { mode: 0o600 });
  const video = new TextEncoder().encode("sealed synthetic video; reader must not load it");
  await writeFile(join(root, "capture.mp4"), video, { mode: 0o600 });
  const sealed = async (relativePath: string, bytes: Uint8Array): Promise<CaptureSealedFile> => {
    const s = await lstat(join(root, relativePath), { bigint: true });
    return { relativePath, sha256: sha(bytes), identity: { device: String(s.dev), inode: String(s.ino), size: Number(s.size), modifiedNs: String(s.mtimeNs) } };
  };
  const contextFile = await sealed("facts/context.json", ctx);
  const videoFile = await sealed("capture.mp4", video);
  const strokeBytes = includeStroke ? enc({ schema: "apparent.capture-strokes/1", identity: factIdentity, coordinates: "target-normalized-10000",
    surfaces: [{ surfaceId: "screen-a", transformId: "transform-a", width: 10, height: 10, target: "capture-rectangle" }],
    strokes: [{ id: "1".repeat(32), surface: "screen", surfaceId: "screen-a", transformId: "transform-a", startMs: 0, endMs: 100, acceptedAtMs: 100, rgb: [0, 0, 0], points: [{ timeMs: 50, x: 5000, y: 5000 }] }] }) : undefined;
  if (strokeBytes) await writeFile(join(root, "facts/strokes.json"), strokeBytes, { mode: 0o600 });
  const strokeFile = strokeBytes ? await sealed("facts/strokes.json", strokeBytes) : undefined;
  const payload = { schema: "apparent.capture-materials-payload/1", identity, bundleId: expected.bundleId, root, leaseId: expected.leaseId,
    clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 1000, videoOffsetMs: 0 },
    confirmedSource: source, invocation, files: [contextFile, videoFile, ...(strokeFile ? [strokeFile] : [])], video: { file: "capture.mp4", width: 10, height: 10, durationMs: 1000 },
    facts: { context: { file: contextFile.relativePath, schema: "apparent.capture-context/1" }, ...(strokeFile ? { acceptedStrokes: { file: strokeFile.relativePath, schema: "apparent.capture-strokes/1" } } : {}) },
    collection: { context: "collected", interactions: "unavailable", candidates: "unavailable", transcript: "unavailable", acceptedStrokes: strokeFile ? "collected" : "unavailable" },
    warnings: [], safety: { nativeRemoteDelivery: "forbidden", safeForProcessing } };
  const payloadBytes = new TextEncoder().encode(JSON.stringify(payload, null, 2) + "\n"); await writeFile(join(root, "materials.json"), payloadBytes, { mode: 0o600 });
  const payloadFile = await sealed("materials.json", payloadBytes);
  const manifest: MaterialsManifest = { schema: "apparent.capture-materials/1", key: { ...identity, revision: sha(payloadBytes) }, root, leaseId: expected.leaseId, payloadFile };
  return { root, manifest, expected: { ...expected, key: manifest.key }, ctx, strokeBytes, payloadBytes };
}

test("retained materials reader preserves validated observed stroke RGB bytes for assembly", async () => {
  const f = await fixture(identity, true, true);
  try {
    const result = await readCaptureMaterials({ manifest: f.manifest, expected: f.expected, trustedRoot: f.root });
    assert.deepEqual(result.materials.payload.collection.acceptedStrokes, "collected");
    assert.deepEqual([...result.assemblyFacts.acceptedStrokes!.bytes], [...f.strokeBytes!]);
    assert.deepEqual(JSON.parse(new TextDecoder().decode(result.assemblyFacts.acceptedStrokes!.bytes)).strokes[0].rgb, [0, 0, 0]);
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test("reads SDK-validated retained bytes while preserving unavailable facts and has no effects", async () => {
  const f = await fixture();
  try {
    await rm(join(f.root, "capture.mp4")); // Metadata loading must not read or require media files.
    const before = await lstat(join(f.root, "materials.json"), { bigint: true });
    const input = { manifest: f.manifest, expected: f.expected, trustedRoot: f.root };
    const pending = readCaptureMaterials(input);
    input.manifest = { ...f.manifest, root: "/changed-after-call" };
    input.expected = { ...f.expected, leaseId: "9".repeat(32) };
    const result = await pending;
    assert.deepEqual(result.payloadBytes, f.payloadBytes);
    assert.equal(sha(result.payloadBytes), result.materials.manifest.key.revision);
    assert.equal(result.materials.payload.clock.durationMs, 1000);
    assert.equal(result.materials.payload.collection.interactions, "unavailable");
    assert.deepEqual([...result.facts.context!.bytes], [...f.ctx]);
    assert.deepEqual(Object.keys(result.assemblyFacts), ["context"]);
    const after = await lstat(join(f.root, "materials.json"), { bigint: true });
    assert.deepEqual([after.dev, after.ino, after.size, after.mtimeNs], [before.dev, before.ino, before.size, before.mtimeNs]);
    assert.deepEqual((await (await import("node:fs/promises")).readdir(f.root)).sort(), ["facts", "materials.json"]);
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test("rejects changed bytes, foreign identity, stale facts, aliases, symlinks and limits", async () => {
  const f = await fixture();
  try {
    const matched = { manifest: f.manifest, expected: f.expected, trustedRoot: f.root };
    assert.equal((await readCaptureMaterials(matched)).materials.payload.bundleId, expected.bundleId);
    await assert.rejects(readCaptureMaterials({ ...matched, manifest: { ...f.manifest, payloadFile: { ...f.manifest.payloadFile, sha256: "9".repeat(64) } } }), /hash mismatch/i);
    await assert.rejects(readCaptureMaterials({ ...matched, expected: { ...f.expected, key: { ...f.expected.key, captureId: "9".repeat(32) } } }), /identity/i);
    await assert.rejects(readCaptureMaterials({ ...matched, limits: { maxFileBytes: 8, maxAggregateBytes: 8 } }), /byte limit/i);
    await assert.rejects(readCaptureMaterials({ ...matched, limits: { maxFileBytes: f.payloadBytes.length, maxAggregateBytes: f.payloadBytes.length + f.ctx.length - 1 } }), /byte limit/i);
    await assert.rejects(readCaptureMaterials({ ...matched, manifest: { ...f.manifest, payloadFile: { ...f.manifest.payloadFile, relativePath: "../materials.json" } } }), /escaping/i);
    const fact = join(f.root, "facts/context.json"); await writeFile(fact, "{}", { mode: 0o600 });
    await assert.rejects(readCaptureMaterials(matched), /identity changed|hash mismatch/i);
  } finally { await rm(f.root, { recursive: true, force: true }); }
  const g = await fixture();
  const alias = `${g.root}-alias`;
  try {
    await symlink(g.root, alias);
    await assert.rejects(readCaptureMaterials({ manifest: g.manifest, expected: g.expected, trustedRoot: alias }), /root|alias|symlink/i);
    await assert.rejects(readCaptureMaterials({ manifest: { ...g.manifest, root: `${g.root}/.` }, expected: g.expected, trustedRoot: g.root }), /root|alias/i);
  } finally { await rm(alias, { force: true }); await rm(g.root, { recursive: true, force: true }); }
  const stale = await fixture({ captureId: "9".repeat(32), materialsId: identity.materialsId });
  try {
    await assert.rejects(readCaptureMaterials({ manifest: stale.manifest, expected: stale.expected, trustedRoot: stale.root }), /identity|capture|fact/i);
  } finally { await rm(stale.root, { recursive: true, force: true }); }
  const linked = await fixture();
  try {
    const actualDir = join(linked.root, "facts"); const otherDir = join(linked.root, "facts-real");
    await (await import("node:fs/promises")).rename(actualDir, otherDir); await symlink(otherDir, actualDir);
    await assert.rejects(readCaptureMaterials({ manifest: linked.manifest, expected: linked.expected, trustedRoot: linked.root }), /symlink/i);
  } finally { await rm(linked.root, { recursive: true, force: true }); }
  const leaf = await fixture();
  try {
    const path = join(leaf.root, "materials.json"); const actual = join(leaf.root, "materials-real.json");
    await (await import("node:fs/promises")).rename(path, actual); await symlink(actual, path);
    await assert.rejects(readCaptureMaterials({ manifest: leaf.manifest, expected: leaf.expected, trustedRoot: leaf.root }), /regular|symlink/i);
  } finally { await rm(leaf.root, { recursive: true, force: true }); }
});


test("unsafe processing and multiply-linked metadata fail closed without changing source bytes", async () => {
  const unsafe = await fixture(identity, false);
  try {
    await assert.rejects(readCaptureMaterials({ manifest: unsafe.manifest, expected: unsafe.expected, trustedRoot: unsafe.root }), /safety/i);
  } finally { await rm(unsafe.root, { recursive: true, force: true }); }
  const linked = await fixture();
  try {
    await link(join(linked.root, "materials.json"), join(linked.root, "payload-alias.json"));
    await assert.rejects(readCaptureMaterials({ manifest: linked.manifest, expected: linked.expected, trustedRoot: linked.root }), /regular private path/i);
    assert.deepEqual(await (await import("node:fs/promises")).readFile(join(linked.root, "materials.json")), Buffer.from(linked.payloadBytes));
  } finally { await rm(linked.root, { recursive: true, force: true }); }
});

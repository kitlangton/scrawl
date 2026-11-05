import assert from "node:assert/strict";
import { chmod, copyFile, link, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { decodeSelectedFrames, assembleProcessingOutput, type DecodeSelection } from "../../src/backend/processing/index.ts";
import { RecordMediaJob, processCompletedBundle, type ManagedMediaClient, type RecordProcessingFunctions } from "../../src/backend/processing/media-consumer.ts";
const processing: RecordProcessingFunctions = { decodeSelectedFrames, assembleProcessingOutput };
type NativeTestMediaClient = ManagedMediaClient & {
  handshake(): Promise<{ implementation: "native" | "test-double"; capabilities: { decodeFrames: boolean } }>;
  close(): Promise<unknown>;
};
type NativeSdkConsumer = {
  MediaClient: new (transport: unknown) => NativeTestMediaClient;
  createMediaJobId(): string;
  launchMediaWorker(executablePath: string, runtimeDirectory: string): unknown;
};

async function packedMediaSdk() {
  const root = process.env.APPARENT_ENGINE_SDK_MEDIA_CONSUMER;
  assert.ok(root, "set APPARENT_ENGINE_SDK_MEDIA_CONSUMER to a clean consumer with the packed SDK installed");
  const mediaPath = `${root}/node_modules/@apparent/engine-sdk/dist/src/media.js`;
  return import(pathToFileURL(mediaPath).href);
}

const selection = (root: string): DecodeSelection => ({
  frames: [{ index: 3, timeMs: 1000, text: "screen changed" }], jobId: "a".repeat(32),
  source: { bundleId: "synthetic.apparent", bundlePath: join(root, "bundle"), videoPath: join(root, "bundle", "recording.mp4"), videoOffsetMs: 0,
    identity: { device: "1", inode: "2", size: 10, modifiedNs: "3" } },
  outputDirectory: join(root, "output"), limits: { maxFrames: 1, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 },
});

test("rejected decode remains durably owned until authoritative inspect", async () => {
  const root = await mkdtemp(join(tmpdir(), "record-media-job-")); await chmod(root, 0o700);
  try {
    const input = selection(root); await mkdir(input.outputDirectory, { mode: 0o700 }); let inspected = 0; let cancelled = 0;
    let decodeCalls = 0;
    const client: ManagedMediaClient = {
      async decode() { decodeCalls++; throw new Error("transport timeout"); },
      async inspect(request) { inspected++; assert.equal(request.jobId, input.jobId); return { jobId: input.jobId, state: "decoding" }; },
      async cancel(jobId) { cancelled++; assert.equal(jobId, input.jobId); return "cancelling"; },
    };
    const journal = join(root, "job.json"); const job = await RecordMediaJob.create(client, processing, journal, input);
    await assert.rejects(job.decode(input), /transport timeout/);
    await assert.rejects(job.decode(input), /requires reconciliation/);
    assert.equal(decodeCalls, 1);
    assert.equal(job.state, "submitted");
    assert.equal(JSON.parse(await readFile(journal, "utf8")).request.jobId, input.jobId);
    const reopened = await RecordMediaJob.reopen(client, processing, journal);
    assert.equal((await reopened.inspect()).state, "decoding");
    assert.equal(await reopened.cancel(), "cancelling");
    assert.equal(inspected, 1); assert.equal(cancelled, 1);
    await assert.rejects(reopened.cleanupJournal(), /unresolved/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("durable request is isolated from caller mutations and busy commands", async () => {
  const root = await mkdtemp(join(tmpdir(), "record-media-owned-"));
  try {
    const input = selection(root), original = structuredClone(input); await mkdir(input.outputDirectory, { mode: 0o700 });
    let calls = 0; let finish!: () => void;
    const pending = new Promise<void>(resolve => { finish = resolve; });
    const client: ManagedMediaClient = {
      async decode(request) { calls++; assert.equal(request.source.bundleId, original.source.bundleId); await pending; throw new Error("timeout"); },
      async inspect() { return { jobId: input.jobId, state: "decoding" }; }, async cancel() { return "cancelling"; },
    };
    const job = await RecordMediaJob.create(client, processing, join(root, "job.json"), input);
    input.source.bundleId = "changed"; input.limits.maxFrames = 99;
    await assert.rejects(job.decode(input), /does not match/);
    const first = job.decode(original);
    await assert.rejects(job.decode(original), /in progress/);
    await assert.rejects(job.inspect(), /in progress/);
    // Cancellation remains available while a decode response is pending.
    assert.equal(await job.cancel(), "cancelling");
    finish(); await assert.rejects(first, /timeout/); assert.equal(calls, 1);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("reopen rejects invalid journal states and inconsistent inventories", async () => {
  const root = await mkdtemp(join(tmpdir(), "record-media-validation-"));
  try {
    const input = selection(root); await mkdir(input.outputDirectory, { mode: 0o700 });
    const client: ManagedMediaClient = { async decode() { throw new Error(); }, async inspect() { return { jobId: input.jobId, state: "decoding" }; }, async cancel() { return "cancelling"; } };
    const journal = join(root, "job.json"); await RecordMediaJob.create(client, processing, journal, input);
    const original = JSON.parse(await readFile(journal, "utf8"));
    for (const change of [{ state: "garbage" }, { state: "complete", frames: [] }, { state: "failed" }, { selection: [] }, { request: { ...original.request, requestedBundleMs: [999] } }]) {
      await writeFile(journal, JSON.stringify({ ...original, ...change }));
      await assert.rejects(RecordMediaJob.reopen(client, processing, journal), /processing journal/);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("foreign inspection and failed terminal persistence cannot release ownership", async () => {
  const root = await mkdtemp(join(tmpdir(), "record-media-inspect-"));
  try {
    const input = selection(root); await mkdir(input.outputDirectory, { mode: 0o700 });
    let foreign = true;
    const client: ManagedMediaClient = { async decode() { throw new Error(); }, async inspect() { return { jobId: foreign ? "b".repeat(32) : input.jobId, state: "cancelled", completedCount: 0, error: { code: "cancelled" } }; }, async cancel() { return "cancelled"; } };
    const journal = join(root, "job.json"), alias = join(root, "alias.json");
    const job = await RecordMediaJob.create(client, processing, journal, input);
    await assert.rejects(job.inspect(), /mismatched job/); assert.equal(job.state, "submitted");
    foreign = false; await link(journal, alias);
    await assert.rejects(job.inspect(), /private owned file/);
    await rm(alias); assert.equal(job.state, "submitted");
    await assert.rejects(job.cleanupJournal(), /unresolved/);
    await job.inspect(); assert.equal(job.state, "cancelled"); await job.cleanupJournal();
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("native packed SDK completed-bundle consumer decodes real images", async t => {
  const entry = process.env.APPARENT_ENGINE_FRAMES_BIN;
  const sdkRoot = process.env.APPARENT_ENGINE_SDK_MEDIA_CONSUMER;
  if (!entry || !sdkRoot) { t.skip("requires the exact packed SDK install and admitted Linux media entry"); return; }
  const { execFileSync } = await import("node:child_process");
  const { createHash } = await import("node:crypto");
  const { stat } = await import("node:fs/promises");
  const root = await mkdtemp(join(tmpdir(), "record-media-native-")); await chmod(root, 0o700);
  const runtime = join(root, "runtime"), bundle = join(root, "fixture.apparent"), output = join(root, "output");
  await mkdir(runtime, { mode: 0o700 }); await mkdir(bundle, { mode: 0o700 }); await mkdir(output, { mode: 0o700 });
  const videoPath = join(bundle, "recording.mp4");
  let client: NativeTestMediaClient | undefined;
  try {
    execFileSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=10:duration=2", "-an", "-c:v", "libx264", "-threads", "1", "-pix_fmt", "yuv420p", "-movflags", "+faststart", videoPath], { timeout: 15000 });
    await chmod(videoPath, 0o600);
    await writeFile(join(bundle, "apparent.json"), JSON.stringify({ version: 1, id: "fixture.apparent", durationMs: 2000, video: "recording.mp4", videoOffsetMs: 0 }), { mode: 0o600 });
    const file = await stat(videoPath, { bigint: true });
    const sdk = await packedMediaSdk() as NativeSdkConsumer;
    client = new sdk.MediaClient(sdk.launchMediaWorker(entry, runtime));
    const handshake = await client.handshake(); assert.equal(handshake.implementation, "native"); assert.equal(handshake.capabilities.decodeFrames, true);
    const source = { bundleId: "fixture.apparent", bundlePath: bundle, videoPath, videoOffsetMs: 0,
      identity: { device: String(file.dev), inode: String(file.ino), size: Number(file.size), modifiedNs: String(file.mtimeNs) } };
    const input: DecodeSelection = { frames: [{ index: 1, timeMs: 500, text: "synthetic chart" }, { index: 2, timeMs: 1500 }], jobId: sdk.createMediaJobId(), source, outputDirectory: output,
      limits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 } };
    const journal = join(root, "processing-job.json");
    const job = await RecordMediaJob.create(client, processing, journal, input);
    const { frames, output: semantic } = await processCompletedBundle(job, processing, input, { durationMs: 2000, bundlePath: bundle, transcript: { text: "Synthetic transcript", segments: [] }, videoPath });
    assert.equal(job.state, "complete"); assert.deepEqual(frames.map(x => x.index), [1, 2]);
    for (const frame of frames) { const bytes = await readFile(frame.absolutePath); assert.equal(bytes.subarray(0, 8).toString("hex"), "89504e470d0a1a0a"); assert.equal(createHash("sha256").update(bytes).digest("hex"), (await client.inspect({ jobId: input.jobId, source, requestedBundleMs: [500, 1500], outputDirectory: output, limits: input.limits })).frames.find((x: any) => x.path === frame.absolutePath).sha256); }
    const inspected = await job.inspect(); assert.equal(inspected.state, "complete");
    assert.match(semantic.summaryMarkdown, /Synthetic transcript/); assert.doesNotMatch(semantic.summaryMarkdown, /synthetic chart/);
    assert.match(semantic.summaryMarkdown, new RegExp(bundle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
    const evidence = process.env.APPARENT_RECORD_MEDIA_EVIDENCE;
    if (evidence) {
      const evidenceFrames = evidence.replace(/\.json$/u, "-frames"); await mkdir(evidenceFrames, { recursive: true, mode: 0o700 });
      const inspectedFrames = await Promise.all(frames.map(async (frame, i) => {
        const bytes = await readFile(frame.absolutePath); const path = join(evidenceFrames, `frame-${i + 1}.png`);
        await copyFile(frame.absolutePath, path); await chmod(path, 0o600);
        return { path, width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), sha256: createHash("sha256").update(bytes).digest("hex") };
      }));
      await writeFile(evidence, JSON.stringify({ source: "synthetic H264 completed bundle", engineSourceSha: "463f0053908a9c6030d3c5665aacd10089884aa9", engineArtifactSha256: "a240c185a67327263b1c09159e1a2afae0bec6d71883489cfb92d8059d98ea0e", sdkArtifactSha256: "8623340ee9b62a77b3bef85fcb27f319a8e25581ed336cff62187a633f4afb94", sdkConsumerRoot: sdkRoot, workerEntry: entry, jobId: input.jobId, frames: inspectedFrames, semanticText: semantic.summaryMarkdown }, null, 2), { mode: 0o600 });
    }
    await job.cleanupJournal();
  } finally { await client?.close(); await rm(root, { recursive: true, force: true }); }
});

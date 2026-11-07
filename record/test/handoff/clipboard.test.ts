import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { test } from "node:test";
import { createRecordLocalClipboardTextReadbackPort, createRecordLocalClipboardPort, type NativeClipboardPortInput } from "../../dist/backend/host/native/index.js";
import { formatLocalHandoff, type LocalHandoffInput } from "../../dist/backend/handoff/index.js";

const identity = { captureId: "capture-1", artifactId: "artifact-1" };
const handoff: LocalHandoffInput = {
  savedBundle: { status: "saved", bundlePath: "/synthetic/capture.apparent" },
  processing: {
    durationMs: 2_000,
    bundlePath: "/synthetic/capture.apparent",
    transcript: { text: "Change the title", segments: [{ startMs: 400, text: "Change the title" }] },
    frames: [{ index: 1, timeMs: 500, absolutePath: "/synthetic/capture.apparent/frames/1.png", text: "A title" }],
    context: { project: "synthetic fixture" },
  },
  requestVideoProof: false,
};
const state = { processEpoch: "epoch-1", revision: 7 };

function resolution(value: LocalHandoffInput = handoff) {
  return { ...identity, handoff: value };
}

function harness(options: {
  resolve?: () => Promise<ReturnType<typeof resolution> | null>;
  snapshot?: () => Promise<typeof state>;
  write?: (text: string, observed: typeof state) => Promise<unknown>;
  maximumTextBytes?: number;
} = {}) {
  const calls: Array<{ text: string; observed: typeof state }> = [];
  let snapshots = 0;
  let shutdowns = 0;
  const clipboard = {
    async writeText(text: string, observed: typeof state) {
      calls.push({ text, observed });
      return options.write ? options.write(text, observed) : applied(text);
    },
    close() { shutdowns++; },
  };
  const nativeHost = {
    async readSnapshot() {
      snapshots++;
      return options.snapshot ? options.snapshot() : state;
    },
    close() { shutdowns++; },
  };
  return {
    calls,
    get snapshots() { return snapshots; },
    get shutdowns() { return shutdowns; },
    port: createRecordLocalClipboardPort({
      // Deliberately malformed replies exercise the runtime boundary; public producer conformance is checked separately.
      clipboard: clipboard as NativeClipboardPortInput,
      nativeHost,
      resolveHandoff: options.resolve ?? (async () => resolution()),
      maximumTextBytes: options.maximumTextBytes ?? 21_800,
    }),
  };
}

function applied(text: string) {
  return {
    status: "applied",
    digest: createHash("sha256").update(text, "utf8").digest("hex"),
    byteLength: Buffer.byteLength(text, "utf8"),
  } as const;
}

test("copies the exact formatted text once against a fresh native snapshot", async () => {
  const h = harness();
  const expectedText = formatLocalHandoff(handoff).text;
  const result = await h.port.copyLocal(identity);
  assert.deepEqual(result, {
    status: "applied", ...identity,
    digest: createHash("sha256").update(expectedText).digest("hex"),
    byteLength: Buffer.byteLength(expectedText, "utf8"),
  });
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0]?.text, expectedText);
  assert.deepEqual(h.calls[0]?.observed, state);
  assert.equal(h.snapshots, 1);
  assert.equal(h.shutdowns, 0);
  assert.equal("text" in result || "bundlePath" in result, false);
});

test("refuses missing, mismatched, and inconsistent Saved facts before native effects", async () => {
  const missing = harness({ resolve: async () => null });
  assert.equal((await missing.port.copyLocal(identity)).status, "refused");
  assert.equal(missing.snapshots, 0);
  assert.equal(missing.calls.length, 0);

  const mismatch = harness({ resolve: async () => ({ ...resolution(), artifactId: "other" }) });
  assert.deepEqual(await mismatch.port.copyLocal(identity), { status: "refused", ...identity, reason: "identity-mismatch" });
  assert.equal(mismatch.snapshots, 0);
  assert.equal(mismatch.calls.length, 0);

  const wrongPath = structuredClone(handoff);
  wrongPath.savedBundle.bundlePath = "/synthetic/other.apparent";
  const inconsistent = harness({ resolve: async () => resolution(wrongPath) });
  assert.deepEqual(await inconsistent.port.copyLocal(identity), { status: "refused", ...identity, reason: "identity-mismatch" });
  assert.equal(inconsistent.snapshots, 0);
  assert.equal(inconsistent.calls.length, 0);
});

test("preserves native refusal and never retries unknown or invalid receipts", async () => {
  const refused = harness({ write: async () => ({ status: "refused", reason: "busy" }) });
  assert.deepEqual(await refused.port.copyLocal(identity), { status: "refused", ...identity, reason: "busy" });
  assert.equal(refused.calls.length, 1);

  for (const write of [
    async () => { throw new Error("private failure detail"); },
    async (text: string) => ({ ...applied(text), digest: "A".repeat(64) }),
    async (text: string) => ({ ...applied(text), byteLength: Buffer.byteLength(text) + 1 }),
    async () => ({ status: "applied", digest: "bad", byteLength: 1, private: "ignored" }),
  ]) {
    const unknown = harness({ write: write as (text: string, observed: typeof state) => Promise<unknown> });
    assert.deepEqual(await unknown.port.copyLocal(identity), { status: "outcome-unknown", ...identity });
    assert.equal(unknown.calls.length, 1);
  }
});

test("uses UTF-8 byte counts and refuses text outside exact valid bounded input", async () => {
  const unicode = structuredClone(handoff);
  unicode.processing.transcript = { text: "🙂", segments: [{ startMs: 1, text: "🙂" }] };
  const fullText = formatLocalHandoff(unicode).text;
  const exactLimit = Buffer.byteLength(fullText, "utf8");
  const exact = harness({ resolve: async () => resolution(unicode), maximumTextBytes: exactLimit });
  assert.equal((await exact.port.copyLocal(identity)).status, "applied");
  assert.equal(exact.calls[0]?.text, fullText);

  const over = harness({ resolve: async () => resolution(unicode), maximumTextBytes: exactLimit - 1 });
  assert.deepEqual(await over.port.copyLocal(identity), { status: "refused", ...identity, reason: "too-large" });
  assert.equal(over.snapshots, 0);
  assert.equal(over.calls.length, 0);

  for (const badText of ["bad\0text", "unpaired \ud800", "unpaired \udfff"]) {
    const invalid = structuredClone(handoff);
    invalid.processing.transcript = { text: badText, segments: [{ startMs: 1, text: badText }] };
    const h = harness({ resolve: async () => resolution(invalid) });
    assert.deepEqual(await h.port.copyLocal(identity), { status: "refused", ...identity, reason: "invalid-text" });
    assert.equal(h.snapshots, 0);
    assert.equal(h.calls.length, 0);
  }

  const huge = structuredClone(handoff);
  const hugeText = "x".repeat(22_000);
  huge.processing.transcript = { text: hugeText, segments: [{ startMs: 1, text: hugeText }] };
  const tooLarge = harness({ resolve: async () => resolution(huge), maximumTextBytes: 21_800 });
  assert.deepEqual(await tooLarge.port.copyLocal(identity), { status: "refused", ...identity, reason: "too-large" });
  assert.equal(tooLarge.calls.length, 0);
});

test("isolates caller and resolver objects across asynchronous boundaries", async () => {
  let finishResolve!: (value: ReturnType<typeof resolution>) => void;
  const pendingResolution = new Promise<ReturnType<typeof resolution>>(resolve => { finishResolve = resolve; });
  let finishWrite!: (value: unknown) => void;
  const pendingWrite = new Promise<unknown>(resolve => { finishWrite = resolve; });
  const mutableHandoff = structuredClone(handoff);
  const mutableResolution = resolution(mutableHandoff);
  const h = harness({
    resolve: async () => pendingResolution,
    write: async () => pendingWrite,
  });
  const caller = { ...identity };
  const pending = h.port.copyLocal(caller);
  caller.captureId = "mutated-capture";
  caller.artifactId = "mutated-artifact";
  finishResolve(mutableResolution);
  while (h.calls.length === 0) await new Promise(resolve => setImmediate(resolve));
  const expected = formatLocalHandoff(handoff).text;
  mutableResolution.artifactId = "changed";
  mutableHandoff.processing.transcript.text = "mutated after dispatch";
  finishWrite(applied(h.calls[0]!.text));
  assert.equal(h.calls[0]?.text, expected);
  assert.deepEqual(await pending, {
    status: "applied", ...identity,
    digest: createHash("sha256").update(expected).digest("hex"),
    byteLength: Buffer.byteLength(expected, "utf8"),
  });
});

test("snapshot failure is a known unavailable refusal before dispatch", async () => {
  const h = harness({ snapshot: async () => { throw new Error("private snapshot detail"); } });
  assert.deepEqual(await h.port.copyLocal(identity), { status: "refused", ...identity, reason: "unavailable" });
  assert.equal(h.calls.length, 0);
});


test("public clipboard adapter composes with the provider and pill using controlled Saved and native receipts", async () => {
  const { createRecordCapabilityProvider } = await import("@apparent/record/capabilities");
  const { createRecordClientPillHost, disposeWorkflowPillHost } = await import("@apparent/record/ui");
  const h = harness();
  const startup = { state: "ready" as const, knownRequestIds: [] };
  const state = { phase: "idle" as const, readiness: { ready: true as const } };
  const report = { requestIds: [], storageUnavailable: false, reconciliation: "complete" as const };
  const client = createRecordCapabilityProvider({
    providerEpoch: "controlled-copy-provider", clipboard: h.port,
    runtime: { state, startup, subscribe: () => () => {}, dispatch: async () => state, reconcileStartup: async () => startup, unresolvedOwnership: () => report, dispose: () => report },
    captureFacts: { readFacts: async () => ({ version: 1, providerEpoch: "controlled-output", revision: 1, nextShortcutAdmission: { state: "unavailable" }, captures: [{
      ...identity, phase: "saved", delivery: "unknown", outcomes: [{ kind: "save", at: "2026-11-06T12:00:00Z", ok: true }],
    }] }) },
  });
  await new Promise(resolve => setImmediate(resolve));
  const host = createRecordClientPillHost(client, { createRequestId: () => "public-copy-action" });
  const view = (await host.read())!;
  await host.act(view.revision, "copy");
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0]!.text, formatLocalHandoff(handoff).text);
  assert.equal((await host.read())!.notice, "Handoff copied");
  const receipt = client.current().captures[0]!.outcomes.find(value => value.kind === "copy");
  assert.equal(receipt && "evidence" in receipt && receipt.evidence, "native-write-readback");
  assert.equal(receipt && "requestId" in receipt && receipt.requestId, "public-copy-action");
  disposeWorkflowPillHost(host); client.detach();
  assert.equal(h.shutdowns, 0, "neither consumer closes caller-owned native ports");
});

 test("separate transient text readback validates exact bytes and never accepts legacy applied", async () => {
  for (const mode of ["written-readback", "applied", "outcome-unknown", "wrong-digest", "wrong-length", "extra", "throw", "refused"] as const) {
    let writes=0;
    const port=createRecordLocalClipboardTextReadbackPort({nativeHost:{readSnapshot:async()=>state},maximumTextBytes:21800,resolveHandoff:async()=>resolution(),clipboard:{async writeTextReadback(text,observed){
      writes++; assert.deepEqual(observed,state); assert.equal(text,formatLocalHandoff(handoff).text);
      if(mode==="throw")throw new Error("private");
      if(mode==="refused")return {status:"refused",reason:"stale"};
      if(mode==="outcome-unknown")return {status:mode};
      return {status:mode==="applied"?"applied":"written-readback",digest:mode==="wrong-digest"?"b".repeat(64):createHash("sha256").update(text).digest("hex"),byteLength:Buffer.byteLength(text)+(mode==="wrong-length"?1:0),...(mode==="extra"?{ownership:true}:{})} as never;
    }}});
    const result=await port.copyLocal(identity);
    assert.equal(result.status,mode==="written-readback"?mode:mode==="refused"?"refused":"outcome-unknown"); assert.equal(writes,1);
    assert.doesNotMatch(JSON.stringify(result),/ownership|private/);
  }
 });
 test("transient readback refuses missing Saved identity and oversize before native effects",async()=>{
  for(const missing of [true,false]){let writes=0;const port=createRecordLocalClipboardTextReadbackPort({nativeHost:{readSnapshot:async()=>state},maximumTextBytes:1,resolveHandoff:async()=>missing?null:resolution(),clipboard:{async writeTextReadback(){writes++;return {status:"outcome-unknown"};}}});
  assert.equal((await port.copyLocal(identity)).status,"refused");assert.equal(writes,0);}
 });

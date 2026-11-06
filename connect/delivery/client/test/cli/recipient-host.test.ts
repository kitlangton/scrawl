import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { runRecipientHost, recipientFailureReceipt, type RecipientHostOptions, type RecipientHostResources } from "../../src/cli/recipient-host.ts";
import { parseRecipientCommand, publishVerified } from "../../src/cli/recipient.ts";

const id = "a".repeat(32), sourceSha = "b".repeat(40), manifestSha = "c".repeat(64);
const supported = process.arch === "x64" && ["linux", "darwin"].includes(process.platform);
const target = process.platform === "darwin" ? "macos-x64" as const : "linux-x64" as const;
const remote = { skip: !supported };
const fail = async (): Promise<never> => { throw Error("private diagnostic"); };

test("host failure projection retains a committed path when evidence receipt verification fails", async () => {
  const result = await publishVerified({ captureId: id, server: "https://sync.example", evidenceOnly: true, files: [] }, "/synthetic/output",
    async () => ({ state: "committed", path: "/synthetic/committed.apparent" }));
  assert.equal(result.ok, false); if (result.ok) return;
  assert.deepEqual(recipientFailureReceipt(result), { exitCode: 1, stdout: "", stderr: "publication_unverified\n",
    publication: { state: "committed", path: "/synthetic/committed.apparent" } });
});

test("local inspect preserves options and never resolves remote state, provider or writer", async () => {
  let calls = 0;
  const result = await runRecipientHost(["inspect", "./local.apparent", "--json", "--marks", "--experiment=one", "--experiment", "two"], {
    resolveRemote: fail, openResources: fail,
    async inspect(source, options) {
      calls++; assert.equal(source, "./local.apparent"); assert.equal(options.json, true); assert.equal(options.marks, true);
      assert.deepEqual(options.experiments, ["one", "two"]); assert.equal(options.cwd, process.cwd());
      assert.ok(Object.isFrozen(options) && Object.isFrozen(options.experiments));
      return { stdout: "synthetic inspection\n", stderr: "synthetic warning\n" };
    },
  });
  assert.deepEqual(result, { receipt: { exitCode: 0, stdout: "synthetic inspection\n", stderr: "synthetic warning\n" }, provider: { state: "not_opened" } });
  assert.equal(calls, 1);
});

test("recipient argument forms preserve attached values, separators and lone hyphens", () => {
  assert.deepEqual(parseRecipientCommand(["fetch", id, "--out=--literal", "--wait=+001", "--"]), { kind: "fetch", id, out: "--literal", waitSeconds: 1, media: false });
  assert.deepEqual(parseRecipientCommand(["fetch", "--out", "-", "--", id]), { kind: "fetch", id, out: "-", waitSeconds: 0, media: false });
  assert.deepEqual(parseRecipientCommand(["inspect", "--", "--json"]), { kind: "inspect-local", source: "--json" });
  assert.deepEqual(parseRecipientCommand(["inspect", "-"]), { kind: "inspect-local", source: "-" });
  for (const args of [["fetch", id, "--out", "-x"], ["fetch", id, "--out=x", "--wait=-1"], ["inspect", ""], ["fetch", id, "--out=x", "--wait=901"]])
    assert.throws(() => parseRecipientCommand(args), { category: "invalid_arguments" });
});

test("invalid argv and unsafe publication paths fail before any remote acquisition", async () => {
  for (const args of [["fetch", id], ["inspect", "--json", "--json"], ["inspect", "--experiment"],
    ["fetch", id, "--out", "../outside"], ["fetch", id, "--out", "bad\0path"]]) {
    const result = await runRecipientHost(args, { resolveRemote: fail, openResources: fail, inspect: fail });
    assert.deepEqual(result, { receipt: { exitCode: 2, stdout: "", stderr: "invalid_arguments\n" }, provider: { state: "not_opened" } });
  }
});

test("inspect separator and cancellation retain local-only semantics without diagnostic leaks", async () => {
  const controller = new AbortController();
  const result = await runRecipientHost(["inspect", "--", "--json"], { resolveRemote: fail, openResources: fail, signal: controller.signal,
    async inspect(source, options) { assert.equal(source, "--json"); assert.equal(options.json, false); controller.abort(); return { stdout: "private output", stderr: "" }; } });
  assert.deepEqual(result.receipt, { exitCode: 1, stdout: "", stderr: "cancelled\n" });
  const failed = await runRecipientHost(["inspect"], { resolveRemote: fail, openResources: fail, inspect: fail });
  assert.deepEqual(failed.receipt, { exitCode: 1, stdout: "", stderr: "inspection_failed\n" });
});

async function fixture(run: (f: Awaited<ReturnType<typeof createFixture>>) => Promise<void>) {
  const f = await createFixture(); try { await run(f); } finally {
    f.server.closeAllConnections(); await new Promise<void>((resolve, reject) => f.server.close(error => error ? reject(error) : resolve()));
    await rm(f.root, { recursive: true, force: true });
  }
}
async function createFixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-recipient-host-")), stateDirectory = path.join(root, "state");
  await mkdir(stateDirectory, { mode: 0o700 });
  const files = new Map([["summary.md", Buffer.from("synthetic summary")], ["apparent.json", Buffer.from("{}")], ["contact-sheet.png", Buffer.from([1, 2, 3])]]);
  const manifest = { id, status: "ready", format: "plaintext-v1", files: [...files].map(([path, bytes]) => ({ path, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") })) };
  const counts = { contexts: 0, providers: 0, closes: 0, renewals: 0, requests: 0, inspections: 0, publications: 0 };
  const behavior = { closeFails: false, wrongIdentity: false, inspectFails: false, uncertain: false, writerMismatch: false };
  const server = createServer((request, response) => {
    counts.requests++; assert.equal(request.headers.authorization, "Bearer synthetic-token");
    if (request.url === `/v1/captures/${id}`) response.end(JSON.stringify(manifest));
    else {
      const bytes = files.get(request.url!.slice(`/v1/captures/${id}/files/`.length));
      if (bytes) response.end(bytes); else response.writeHead(404).end();
    }
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert.ok(address && typeof address === "object"); const origin = `http://127.0.0.1:${address.port}`;
  await writeFile(path.join(stateDirectory, "read.credential"), JSON.stringify({ server: origin, scope: "read", token: "synthetic-token", generation: "d".repeat(32) }), { mode: 0o600 });
  const provider: RecipientHostResources = {
    protocol: 1, target, sourceSha, manifestSha256: manifestSha,
    checkAcl: async handle => { await handle.close(); return { state: "accepted" }; },
    lock: async handle => { await handle.close(); let held = true; return { state: "acquired", lease: { signal: new AbortController().signal,
      assertHeld() { assert.ok(held); }, async release() { held = false; } } }; },
    publish: async (handle, source, destination) => {
      try {
        counts.publications++;
        if (behavior.uncertain) return { state: "publication_uncertain" };
        await rename(path.join(root, source), path.join(root, destination)); return { state: "published" };
      } finally { await handle.close(); }
    },
    async close() { counts.closes++; if (behavior.closeFails) throw Error("private retirement diagnostic"); },
  };
  const options: RecipientHostOptions = {
    cwd: root,
    async resolveRemote() {
      counts.contexts++;
      return { directory: stateDirectory, approvedServer: origin, allowLegacyPlaintext: true,
        admission: { artifactDirectory: root, trustedManifestSha256: manifestSha, expectedSourceSha: sourceSha, target },
        writer: { directory: behavior.writerMismatch ? root : stateDirectory, approvedServer: origin, renewRead: async () => { counts.renewals++; return { state: "current" }; } } };
    },
    async openResources(admission) {
      counts.providers++; assert.ok(Object.isFrozen(admission)); assert.equal(admission.trustedManifestSha256, manifestSha);
      return behavior.wrongIdentity ? { ...provider, sourceSha: "e".repeat(40) } : provider;
    },
    async inspect(source, options) {
      counts.inspections++; assert.equal(counts.closes, 0); assert.equal(options.json, true);
      assert.equal(await readFile(path.join(source, "summary.md"), "utf8"), "synthetic summary");
      if (behavior.inspectFails) throw Error("private inspection diagnostic");
      return { stdout: "synthetic rendered inspection\n", stderr: "Evidence snapshot\n" };
    },
  };
  return { root, stateDirectory, origin, options, behavior, counts, server };
}

test("host joins loopback retrieval and injected publication before inspection and retirement", remote, () => fixture(async f => {
  const result = await runRecipientHost(["inspect", id, "--json"], f.options);
  assert.equal(result.receipt.exitCode, 0); assert.equal(result.receipt.stdout, "synthetic rendered inspection\n"); assert.equal(result.provider.state, "retired");
  assert.deepEqual(result.receipt.publication, { state: "committed", path: path.join(f.root, `${id}.evidence.apparent`) });
}));

test("fetch prints the committed path once and does not invoke inspection or a writer", remote, () => fixture(async f => {
  const destination = path.join(f.root, "full.apparent");
  const result = await runRecipientHost(["fetch", id, `--out=${destination}`], f.options);
  assert.deepEqual(result.receipt, { exitCode: 0, stdout: `${destination}\n`, stderr: "", publication: { state: "committed", path: destination } });
  assert.equal(f.counts.inspections, 0); assert.equal(f.counts.renewals, 0); assert.equal(f.counts.publications, 1); assert.equal(f.counts.closes, 1);
}));

test("mismatched writer namespace and provider identity fail closed without credential transport", remote, async () => {
  for (const mode of ["writerMismatch", "wrongIdentity"] as const) await fixture(async f => {
    f.behavior[mode] = true;
    const result = await runRecipientHost(["fetch", id, "--out", path.join(f.root, "out")], f.options);
    assert.equal(result.receipt.exitCode, 1); assert.equal(result.receipt.stderr, "state_unavailable\n"); assert.equal(f.counts.requests, 0);
    assert.equal(f.counts.providers, mode === "writerMismatch" ? 0 : 1); assert.equal(f.counts.closes, mode === "writerMismatch" ? 0 : 1);
  });
});

test("missing trust pins, wrong registered target and unbound writer fail before provider creation", remote, async () => {
  for (const mode of ["pin", "target", "owner", "server"] as const) await fixture(async f => {
    const resolve = f.options.resolveRemote;
    f.options.resolveRemote = async () => {
      const value = await resolve();
      if (mode === "pin") value.admission = { ...value.admission, trustedManifestSha256: "" };
      if (mode === "target") value.admission = { ...value.admission, target: target === "linux-x64" ? "macos-x64" : "linux-x64" };
      if (mode === "owner") value.writer = { ...value.writer, renewRead: undefined as never };
      if (mode === "server") value.writer = { ...value.writer, approvedServer: "https://other.example" };
      return value;
    };
    const result = await runRecipientHost(["fetch", id, "--out", path.join(f.root, "out")], f.options);
    assert.equal(result.receipt.stderr, "state_unavailable\n"); assert.equal(result.provider.state, "not_opened");
    assert.equal(f.counts.providers, 0); assert.equal(f.counts.requests, 0);
  });
});

test("cancellation after resolver or factory acquisition closes only an acquired provider", remote, async () => {
  for (const stage of ["resolve", "open"] as const) await fixture(async f => {
    const controller = new AbortController(); f.options.signal = controller.signal;
    if (stage === "resolve") {
      const resolve = f.options.resolveRemote; f.options.resolveRemote = async () => { const value = await resolve(); controller.abort(); return value; };
    } else {
      const open = f.options.openResources; f.options.openResources = async admission => { const value = await open(admission); controller.abort(); return value; };
    }
    const result = await runRecipientHost(["fetch", id, "--out", path.join(f.root, "out")], f.options);
    assert.equal(result.receipt.stderr, "cancelled\n"); assert.equal(f.counts.requests, 0); assert.equal(f.counts.publications, 0);
    assert.equal(f.counts.closes, stage === "resolve" ? 0 : 1);
  });
});

test("unsupported installed architectures never fall back to a direct provider", { skip: supported }, () => fixture(async f => {
  const result = await runRecipientHost(["fetch", id, "--out", path.join(f.root, "out")], f.options);
  assert.equal(result.receipt.stderr, "state_unavailable\n"); assert.equal(f.counts.providers, 0); assert.equal(f.counts.requests, 0);
}));

test("failed retirement retains ownership and does not retry a committed publication", remote, () => fixture(async f => {
  f.behavior.closeFails = true;
  const result = await runRecipientHost(["fetch", id, "--out", path.join(f.root, "out")], f.options);
  assert.equal(result.receipt.exitCode, 1); assert.equal(result.receipt.publication?.state, "committed");
  assert.equal(result.receipt.stderr, "provider_retirement_failed\n"); assert.equal(result.provider.state, "retained");
  assert.equal(f.counts.publications, 1); assert.equal(f.counts.closes, 1);
  f.behavior.closeFails = false; if (result.provider.state === "retained") await result.provider.retire();
  assert.equal(f.counts.closes, 2); assert.equal(f.counts.publications, 1);
}));

test("uncertain publication is not retried or passed to inspection", remote, () => fixture(async f => {
  f.behavior.uncertain = true;
  const result = await runRecipientHost(["fetch", id, "--out", path.join(f.root, "out")], f.options);
  assert.equal(result.receipt.exitCode, 1); assert.deepEqual(result.receipt.publication, { state: "uncertain" });
  assert.equal(f.counts.publications, 1); assert.equal(f.counts.inspections, 0); assert.equal(f.counts.closes, 1);
}));

test("failed post-download inspection retains the committed destination and closes resources", remote, () => fixture(async f => {
  f.behavior.inspectFails = true;
  const result = await runRecipientHost(["inspect", id, "--json"], f.options);
  assert.equal(result.receipt.exitCode, 1); assert.equal(result.receipt.stderr, "inspection_failed\n");
  assert.deepEqual(result.receipt.publication, { state: "committed", path: path.join(f.root, `${id}.evidence.apparent`) });
  assert.equal(f.counts.publications, 1); assert.equal(f.counts.closes, 1); assert.equal(result.provider.state, "retired");
}));

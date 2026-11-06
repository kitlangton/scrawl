import assert from "node:assert/strict";
import { replacePrivateStateFile, type StateReplacementResources } from "../../src/state/replacement.ts";
import { commitCompletedRenewal } from "../../src/state/renewal-commit.ts";
import { persistRenewalExchange, renewStateCredential } from "../../src/state/renewal-exchange.ts";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { createServer } from "node:http";
import { once } from "node:events";
import { spawn } from "node:child_process";
import { chmod, link, lstat, mkdir, mkdtemp, open, readFile, readdir, realpath, rename, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test, { after } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { encodeArchive, encryptAge, generateAgeIdentity, signEnvelope, splitAgeStream, type Context } from "../../src/recipient/crypto.ts";
import { runRecipientCommandFromState } from "../../src/cli/recipient-state.ts";
import { runRecipientHost } from "../../src/cli/recipient-host.ts";
import { parseRecipientCommand, runRecipientCommand } from "../../src/cli/recipient.ts";
import { publishRecipientCapture, RecipientPublicationError } from "../../src/recipient/publication.ts";
import { readCredentialSnapshot, assertCredentialSnapshotCurrent, CredentialSnapshotError, type NamespaceResources } from "../../src/state/namespace.ts";
import { readStoredCredentialHandle, CredentialReadError } from "../../src/credentials/private-read.ts";
import { decodeRecipientRenewalReply, type RecipientRenewalOwner } from "../../src/state/renewal-owner.ts";
import type { RetrievedRecipientCapture } from "../../src/recipient/retrieve.ts";
import { readQueueSnapshot, QueueStoreError } from "../../src/queue/store.ts";
import { readLocalStatus } from "../../src/cli/status-state.ts";

const target = process.platform === "linux" ? "linux-x64" : process.arch === "arm64" ? "macos-arm64" : "macos-x64";
const artifactPath = process.env.APPARENT_RESOURCE_ARTIFACT;
function absoluteEnv(name: string, value: string | undefined): string {
  assert.ok(value, `${name} must name the prebuilt native resource artifact`);
  assert.ok(path.isAbsolute(value), `${name} must be an absolute path`);
  return value;
}
const artifact = absoluteEnv("APPARENT_RESOURCE_ARTIFACT", artifactPath);
const providerMode = process.env.APPARENT_RESOURCE_PROVIDER ?? "direct";
assert.ok(providerMode === "direct" || providerMode === "installed", "resource provider mode must be explicit and supported");
async function packedResourceSdk() {
  const consumer = process.env.APPARENT_RESOURCE_SDK_CONSUMER;
  absoluteEnv("APPARENT_RESOURCE_SDK_CONSUMER", consumer);
  const entry = `${consumer}/node_modules/@apparent/native-resources/dist/index.js`;
  const installedEntry = `${consumer}/node_modules/@apparent/native-resources/dist/installed.js`;
  const root = await import(pathToFileURL(entry).href);
  const installed = providerMode === "installed" ? await import(pathToFileURL(installedEntry).href) : {};
  type Ports = NamespaceResources & StateReplacementResources & {
    publish(parent: import("node:fs/promises").FileHandle, source: string, destination: string, options?: { signal?: AbortSignal }): Promise<{ state: "published" | "destination_exists" | "refused" | "publication_uncertain" }>;
  };
  return { ...root, ...installed } as {
  createInstalledNativeResources(options: { artifactDirectory: string; trustedManifestSha256: string; expectedSourceSha: string; target: string }): Promise<Ports & { close(): Promise<void>; readonly executable: string; readonly protocol: 1; readonly target: "linux-x64" | "macos-x64"; readonly sourceSha: string; readonly manifestSha256: string }>;
  NativeResources: new (provider: { executable: string; protocol: 1; target: string; sha256: string }) => Ports;
  validateResourceArtifact(directory: string, expectedTarget: string): { sourceSha: string; target: string; executable: string; sha256: string; protocol: number };
};
}
const nativeModule = await packedResourceSdk();
const manifest = nativeModule.validateResourceArtifact(artifact, target);
const expectedSource = process.env.APPARENT_RESOURCE_SOURCE_SHA;
assert.match(expectedSource ?? "", /^[a-f0-9]{40}$/, "APPARENT_RESOURCE_SOURCE_SHA must pin the reviewed producer source");
assert.equal(manifest.sourceSha, expectedSource, "resource artifact must match the reviewed producer source");
assert.equal(manifest.target, target, "resource artifact target must match the running platform");
assert.equal(manifest.protocol, 1);
assert.equal(manifest.executable, "apparent-resource-host");
const executable = path.join(artifact, manifest.executable);
const binarySha = createHash("sha256").update(await readFile(executable)).digest("hex");
assert.equal(binarySha, manifest.sha256, "provider SHA must come from the admitted artifact bytes");
const trustedManifestSha256 = process.env.APPARENT_RESOURCE_MANIFEST_SHA256;
if (providerMode === "installed") assert.match(trustedManifestSha256 ?? "", /^[a-f0-9]{64}$/, "installed mode requires an externally admitted manifest digest");
const installedResources = providerMode === "installed" ? await nativeModule.createInstalledNativeResources({ artifactDirectory: artifact,
  trustedManifestSha256: trustedManifestSha256!, expectedSourceSha: expectedSource!, target }) : undefined;
const resources = providerMode === "installed" ? installedResources! : new nativeModule.NativeResources({ executable, protocol: 1, target: manifest.target, sha256: binarySha });
assert.ok(resources, "installed mode must return a provider; direct fallback is forbidden");
if (installedResources) after(async () => {
  const stagedExecutable = installedResources.executable;
  await installedResources.close();
  await assert.rejects(lstat(stagedExecutable), { code: "ENOENT" });
  assert.equal(createHash("sha256").update(await readFile(executable)).digest("hex"), binarySha, "retirement preserves the original admitted artifact");
});
assert.equal(typeof resources.checkAcl, "function", "native SDK must provide descriptor ACL verification");

const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

test("native queue snapshot uses the stable lease and preserves private job data", async () => fixture(async root => {
  const id = "a".repeat(32), context = { scope: "read" as const, approvedServer: "https://connect.example" };
  const file = path.join(root, `${id}.job`);
  const bytes = Buffer.from(JSON.stringify({ id, bundle: "/synthetic.apparent", server: context.approvedServer,
    generation: "b".repeat(32), created: 10, attempts: 0, nextAttempt: 0, status: "pending", future: { retained: true } }));
  await writeFile(file, bytes, { mode: 0o600 });
  await link(file, path.join(root, "saved-record"));
  const jobs = await readQueueSnapshot(root, context, resources);
  assert.equal(jobs[0]!.id, id); assert.deepEqual(jobs[0]!.future, { retained: true });
  assert.ok(Object.isFrozen(jobs[0]!.future)); assert.deepEqual(await readFile(file), bytes);
  const lockPath = path.join(root, "queue.lock"), before = await lstat(lockPath);
  const acquired = await resources.lock(await open(lockPath, constants.O_RDWR | constants.O_NOFOLLOW), { timeoutMs: 0 });
  assert.equal(acquired.state, "acquired");
  if (acquired.state === "acquired") {
    try { await assert.rejects(readQueueSnapshot(root, context, resources, { lockTimeoutMs: 0 }),
      e => e instanceof QueueStoreError && e.category === "lock_refused"); }
    finally { await acquired.lease.release(); }
  }
  assert.equal((await lstat(lockPath)).ino, before.ino);
  await readQueueSnapshot(root, context, resources);
  const credential = Buffer.from(JSON.stringify({ server: context.approvedServer, scope: "read", token: "synthetic-status-token", generation: "b".repeat(32) }));
  await writeFile(path.join(root, "read.credential"), credential, { mode: 0o600 });
  const status = await readLocalStatus(root, { server: context.approvedServer, serverValid: true, hostedSync: true,
    cloudEnabled: true, uploadConsent: true, encryption: false, nowSeconds: 100 }, resources);
  assert.equal(status.readSignedIn, true); assert.equal(status.uploadSignedIn, false);
  assert.equal(status.queue[0]!.id, id); assert.equal(status.enabled, true);
  assert.ok(!JSON.stringify(status).includes("synthetic-status-token"));
  assert.deepEqual(await readFile(path.join(root, "read.credential")), credential);
  await chmod(file, 0o644);
  await assert.rejects(readQueueSnapshot(root, context, resources), e => e instanceof QueueStoreError && e.category === "unsafe_file");
  assert.deepEqual(await readFile(file), bytes);
}));
function capture(evidenceOnly = false): RetrievedRecipientCapture {
  const entries = [
    ["summary.md", Buffer.from("synthetic recipient summary")],
    ["contact-sheet.png", Buffer.from([1, 2, 3])],
    ["apparent.json", Buffer.from(JSON.stringify({ contactSheet: "contact-sheet.png", keyframes: [{ file: "frames/frame-01.png" }] }))],
    ["frames/frame-01.png", Buffer.from([4, 5, 6])],
  ] as const;
  return {
    captureId: "a".repeat(32), server: "https://connect.example", evidenceOnly,
    files: entries.map(([filePath, bytes]) => ({ path: filePath, bytes: new Uint8Array(bytes), size: bytes.length, sha256: digest(bytes) })),
  };
}
async function fixture<T>(fn: (root: string) => Promise<T>): Promise<T> {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "apparent-connect-publication-native-"));
  await chmod(root, 0o700);
  try { return await fn(root); } finally { await rm(root, { recursive: true, force: true }); }
}
function mode(value: number, expected: number, label: string): void { assert.equal(value & 0o777, expected, `${label} mode`); }
function isCategory(category: string) {
  return (error: unknown) => error instanceof RecipientPublicationError && error.category === category;
}

test("actual native consumer publishes all verified files with private modes and readable bytes", async () => fixture(async root => {
  const destination = path.join(root, "complete.apparent");
  assert.deepEqual(await publishRecipientCapture(capture(), destination, resources), { state: "committed", path: destination });
  assert.deepEqual((await readdir(destination, { recursive: true })).sort(), ["apparent.json", "contact-sheet.png", "frames", "frames/frame-01.png", "summary.md"]);
  assert.equal(await readFile(path.join(destination, "summary.md"), "utf8"), "synthetic recipient summary");
  assert.deepEqual(await readFile(path.join(destination, "frames/frame-01.png")), Buffer.from([4, 5, 6]));
  mode((await lstat(destination)).mode, 0o700, "bundle");
  mode((await lstat(path.join(destination, "frames"))).mode, 0o700, "asset directory");
  for (const file of ["summary.md", "contact-sheet.png", "apparent.json", "frames/frame-01.png"]) mode((await lstat(path.join(destination, file))).mode, 0o600, file);
}));

test("actual native consumer writes the exact evidence-only marker and private files", async () => fixture(async root => {
  const destination = path.join(root, "evidence.apparent");
  assert.deepEqual(await publishRecipientCapture(capture(true), destination, resources), { state: "committed", path: destination, evidenceOnlyReceipt: true });
  assert.deepEqual(JSON.parse(await readFile(path.join(destination, ".evidence-only.json"), "utf8")), {
    version: 1, captureId: "a".repeat(32), server: "https://connect.example", media: "not downloaded",
    guidance: "Use apparent fetch <id> --media --out <new-directory> for the complete recording",
  });
  assert.equal(await readFile(path.join(destination, "frames/frame-01.png")).then(bytes => bytes.length), 3);
  mode((await lstat(destination)).mode, 0o700, "evidence bundle");
  for (const file of ["summary.md", "contact-sheet.png", "apparent.json", ".evidence-only.json"]) mode((await lstat(path.join(destination, file))).mode, 0o600, file);
}));

test("actual native consumer preserves legitimate Unicode and spaces", async () => fixture(async root => {
  const destination = path.join(root, "Sam's review 👋.apparent");
  assert.deepEqual(await publishRecipientCapture(capture(), destination, resources), { state: "committed", path: destination });
  assert.equal(await readFile(path.join(destination, "summary.md"), "utf8"), "synthetic recipient summary");
}));

test("actual native consumer leaves an existing destination sentinel untouched", async () => fixture(async root => {
  const destination = path.join(root, "taken.apparent"); await mkdir(destination, { mode: 0o700 });
  await writeFile(path.join(destination, "sentinel"), "preserve", { mode: 0o600 });
  await assert.rejects(() => publishRecipientCapture(capture(), destination, resources), isCategory("destination_exists"));
  assert.equal(await readFile(path.join(destination, "sentinel"), "utf8"), "preserve");
  assert.deepEqual((await readdir(root)).sort(), ["taken.apparent"]);
}));

test("actual native consumer refuses traversal and symlink parents before publication", async () => fixture(async root => {
  await assert.rejects(() => publishRecipientCapture(capture(), `${root}/../escaped.apparent`, resources), isCategory("invalid_destination"));
  const alias = path.join(root, "alias"); await symlink(root, alias);
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(alias, "escaped.apparent"), resources), error => error instanceof RecipientPublicationError);
  assert.deepEqual(await readdir(root), ["alias"]);
}));

test("actual native consumer honors cancellation before creating output", async () => fixture(async root => {
  const controller = new AbortController(); controller.abort();
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(root, "cancelled.apparent"), resources, { signal: controller.signal }), isCategory("cancelled"));
  assert.deepEqual(await readdir(root), []);

  const afterStage = new AbortController(); let aclChecks = 0;
  const cancelDuringStaging = {
    checkAcl: async (handle: import("node:fs/promises").FileHandle) => {
      const result = await resources.checkAcl(handle);
      // root + each parent component + stage + the first nested asset directory
      if (++aclChecks === path.resolve(root).split(path.sep).filter(Boolean).length + 3) afterStage.abort();
      return result;
    },
    publish: resources.publish.bind(resources),
  };
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(root, "cancelled-after-stage.apparent"), cancelDuringStaging, { signal: afterStage.signal }), isCategory("cancelled"));
  assert.deepEqual(await readdir(root), [], "cancellation during staging cleans only its private stage");
}));

test("actual native consumer accepts a trusted sticky parent and publishes with no replacement", async () => fixture(async root => {
  await chmod(root, 0o1777);
  const destination = path.join(root, "sticky.apparent");
  assert.deepEqual(await publishRecipientCapture(capture(), destination, resources), { state: "committed", path: destination });
  assert.equal(await readFile(path.join(destination, "summary.md"), "utf8"), "synthetic recipient summary");
}));

test("recipient CLI retrieves a synthetic encrypted capture and publishes through the actual native adapter", async () => fixture(async root => {
  const id = "0123456789abcdef0123456789abcdef";
  const accountId = "synthetic-account", senderGrantId = "synthetic-sender";
  const token = "synthetic-read-token";
  const signingSeed = new Uint8Array(32).fill(19), senderSeed = new Uint8Array(32).fill(37);
  const reader = await generateAgeIdentity();
  const contents = [
    { path: "summary.md", bytes: Buffer.from("synthetic CLI handoff\n") },
    { path: "apparent.json", bytes: Buffer.from("{}") },
    { path: "contact-sheet.png", bytes: Buffer.from([7, 8, 9]) },
  ];
  const digestHex = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
  const assets = contents.map(file => ({ path: file.path, size: file.bytes.length, sha256: digestHex(file.bytes) }));
  const routes = new Map<string, Buffer | Record<string, unknown>>();
  const server = createServer((request, response) => {
    assert.equal(request.method, "GET");
    assert.equal(request.headers.authorization, `Bearer ${token}`);
    const body = routes.get(request.url ?? "");
    if (!body) { response.writeHead(404).end(); return; }
    response.writeHead(200, { "content-type": Buffer.isBuffer(body) ? "application/octet-stream" : "application/json" });
    response.end(Buffer.isBuffer(body) ? body : JSON.stringify(body));
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  const origin = `http://127.0.0.1:${address.port}`;
  try {
    const context: Context = { origin, accountId, captureId: id, senderGrantId };
    const archive = encodeArchive({ version: 1, context, files: assets }, contents.map(file => file.bytes));
    const parts = splitAgeStream(await encryptAge(archive, [reader.recipient]));
    const chunks = parts.map((bytes, index) => ({ path: `sealed/part-${String(index).padStart(6, "0")}.age`, bytes }));
    const chunkAssets = chunks.map(file => ({ path: file.path, size: file.bytes.length, sha256: digestHex(file.bytes) }));
    const signed = { version: 1, context, chunks: chunkAssets };
    const envelope = Buffer.from(JSON.stringify({ signed, signature: await signEnvelope(senderSeed, signed) }));
    const transport = [{ path: "envelope.json", bytes: envelope }, ...chunks];
    routes.set(`/v1/captures/${id}`, { id, status: "ready", format: "apparent-encrypted-v1", files: transport.map(file => ({ path: file.path, size: file.bytes.length, sha256: digestHex(file.bytes) })) });
    const key = async (recipient: string, seed: Uint8Array) => ({ version: 1, recipient, signing: Array.from(await ed25519.getPublicKeyAsync(seed), byte => byte.toString(16).padStart(2, "0")).join("") });
    routes.set("/v1/encryption/peers", { accountId, grantId: "reader", scope: "read", devices: [
      { grantId: "reader", scope: "read", key: await key(reader.recipient, signingSeed) },
      { grantId: senderGrantId, scope: "upload", key: await key(reader.recipient, senderSeed) },
    ] });
    for (const file of transport) routes.set(`/v1/captures/${id}/files/${file.path}`, Buffer.from(file.bytes));
    const destination = path.join(root, "cli.apparent");
    const result = await runRecipientCommand(parseRecipientCommand(["fetch", id, "--out", destination]), {
      loadCredential: async () => ({ server: origin, token, grantId: "reader", identity: { age: reader.identity, signing: signingSeed } }),
      publish: (capture, out) => publishRecipientCapture(capture, out, resources),
      inspectLocal: async source => source,
    });
    assert.deepEqual(result, { ok: true, path: destination });
    assert.equal(await readFile(path.join(destination, "summary.md"), "utf8"), "synthetic CLI handoff\n");
    mode((await lstat(destination)).mode, 0o700, "CLI bundle");
    const state = path.join(root, "state"); await mkdir(state, { mode: 0o700 });
    const identityId = "f".repeat(32), expiresAt = Math.floor(Date.now() / 1000) + 3600;
    await writeFile(path.join(state, "read.credential"), JSON.stringify({ server: origin, scope: "read", token, generation: "a".repeat(32), refreshToken: "synthetic-refresh",
      expiresAt, refreshExpiresAt: expiresAt + 3600, grantId: "reader", encryption_identity: identityId }), { mode: 0o600 });
    await writeFile(path.join(state, `${identityId}.identity`), JSON.stringify({ version: 1, age: reader.identity, signing: Buffer.from(signingSeed).toString("hex") }), { mode: 0o600 });
    const stateDestination = path.join(root, "state-cli.apparent");
    const stateResult = await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", stateDestination]), {
      directory: state, approvedServer: origin, resources, inspectLocal: async source => source,
    });
    assert.deepEqual(stateResult, { ok: true, path: stateDestination });
    assert.equal(await readFile(path.join(stateDestination, "summary.md"), "utf8"), "synthetic CLI handoff\n");
    const credentialFile = path.join(state, "read.credential");
    const base = { ...JSON.parse(await readFile(credentialFile, "utf8")), token: "consumed-access", expiresAt: 1, refreshExpiresAt: 1 };
    await writeFile(credentialFile, JSON.stringify(base), { mode: 0o600 });
    await writeFile(path.join(state, "read.renewal"), JSON.stringify({ base, issued: Math.floor(Date.now() / 1000), completed: true }), { mode: 0o600 });
    await writeFile(path.join(state, "read.response"), JSON.stringify({ scope: "read", grantId: "reader", accessToken: token, refreshToken: "rotated-refresh", expiresIn: 3600, refreshExpiresIn: 7200 }), { mode: 0o600 });
    const beforeRecovery = await readFile(credentialFile), recoveredDestination = path.join(root, "recovered-cli.apparent");
    assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", recoveredDestination]), {
      directory: state, approvedServer: origin, resources, inspectLocal: async source => source,
    }), { ok: true, path: recoveredDestination });
    assert.deepEqual(await readFile(credentialFile), beforeRecovery);


  } finally { server.closeAllConnections(); server.close(); await once(server, "close"); }
}));

test("recipient requests use the actual sole writer for due renewal, one GET retry and terminal rejection",
  { skip: !process.env.APPARENT_RECIPIENT_WRITER_BIN }, async () => fixture(async root => {
  const writer = absoluteEnv("APPARENT_RECIPIENT_WRITER_BIN", process.env.APPARENT_RECIPIENT_WRITER_BIN);
  assert.equal(digest(await readFile(writer)), process.env.APPARENT_RECIPIENT_WRITER_SHA256, "pin the externally built writer bytes; no consumer compilation");
  const id = "1".repeat(32), state = path.join(root, "hosted"), config = path.join(root, "config.json");
  await mkdir(state, { mode: 0o700 });
  const configBytes = JSON.stringify({ unknown: "preserved", cloud: { enabled: false, uploadConsent: false } });
  await writeFile(config, configBytes, { mode: 0o600 });
  const upload = JSON.stringify({ server: "https://other.example", token: "untouched-upload" });
  await writeFile(path.join(state, "upload.credential"), upload, { mode: 0o600 });
  let refreshes = 0, metadata = 0, calls = 0, rejectAll = false, midTransfer = false;
  let cancelRefresh: AbortController | undefined;
  let access = "old-access", refresh = "old-refresh";
  const content = Buffer.from("synthetic renewal recipient\n");
  const assets = new Map([["summary.md", content], ["apparent.json", Buffer.from("{}")], ["contact-sheet.png", Buffer.from([1, 2, 3])]]);
  const files = [...assets].map(([path, bytes]) => ({ path, size: bytes.length, sha256: digest(bytes) }));
  const server = createServer(async (request, response) => {
    calls++;
    if (request.url === "/v1/token/refresh") {
      assert.equal(request.method, "POST"); assert.equal(request.headers.authorization, undefined);
      let body = ""; for await (const chunk of request) body += chunk;
      assert.deepEqual(JSON.parse(body), { refreshToken: refresh });
      if (cancelRefresh) { cancelRefresh.abort(); return; } // Hold response until cancellation stops the real writer.
      refreshes++; access = `access-${refreshes}`; refresh = `refresh-${refreshes}`;
      response.end(JSON.stringify({ scope: "read", grantId: "reader", accessToken: access, refreshToken: refresh, expiresIn: 3600, refreshExpiresIn: 7200 })); return;
    }
    assert.equal(request.method, "GET"); assert.equal(request.headers.cookie, undefined);
    if (request.url === `/v1/captures/${id}`) metadata++;
    if (rejectAll || midTransfer && request.url?.endsWith("/summary.md") && refreshes === 1) {
      response.writeHead(401); response.end("private upstream diagnostic"); return;
    }
    assert.equal(request.headers.authorization, `Bearer ${access}`);
    if (request.url === `/v1/captures/${id}`) response.end(JSON.stringify({ id, status: "ready", files }));
    else if (request.url?.startsWith(`/v1/captures/${id}/files/`) && assets.has(request.url.slice(`/v1/captures/${id}/files/`.length))) response.end(assets.get(request.url.slice(`/v1/captures/${id}/files/`.length)));
    else response.writeHead(404).end();
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  const origin = `http://127.0.0.1:${address.port}`, credentialFile = path.join(state, "read.credential");
  const renewRead = writerFixtureOwner(writer, config);
  try {
    await writeFile(credentialFile, JSON.stringify({ server: origin, scope: "read", token: access, refreshToken: refresh,
      generation: "a".repeat(32), grantId: "reader", expiresAt: 1, refreshExpiresAt: Math.floor(Date.now() / 1000) + 7200 }), { mode: 0o600 });
    const options = { directory: state, approvedServer: origin, resources, renewRead, allowLegacyPlaintext: true, inspectLocal: async (source: string) => source };
    const destination = path.join(root, "renewed.apparent"); midTransfer = true;
    assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", destination]), options), { ok: true, path: destination });
    assert.equal(refreshes, 2); assert.equal(metadata, 1, "retry only the rejected file GET, not the whole retrieval");
    assert.equal(await readFile(path.join(destination, "summary.md"), "utf8"), content.toString());
    assert.equal(JSON.parse(await readFile(credentialFile, "utf8")).token, access);
    assert.equal(await readFile(config, "utf8"), configBytes); assert.equal(await readFile(path.join(state, "upload.credential"), "utf8"), upload);
    rejectAll = true; const before = refreshes;
    const rejected = await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", path.join(root, "denied.apparent")]), options);
    assert.deepEqual(rejected, { ok: false, error: { category: "auth_required", message: "auth_required" } });
    assert.equal(refreshes, before + 1); assert.equal(JSON.parse(await readFile(credentialFile, "utf8")).authRequired, true);
    const beforeCalls = calls;
    assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", path.join(root, "again.apparent")]), options), rejected);
    assert.equal(calls, beforeCalls, "authRequired never starts another request or renewal");
    assert.equal(await readFile(config, "utf8"), configBytes); assert.equal(await readFile(path.join(state, "upload.credential"), "utf8"), upload);
    const expired = { ...JSON.parse(await readFile(credentialFile, "utf8")), expiresAt: 1, authRequired: false };
    await writeFile(credentialFile, JSON.stringify(expired), { mode: 0o600 });
    const beforeCancellation = await readFile(credentialFile), beforeMetadata = metadata;
    cancelRefresh = new AbortController();
    assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", path.join(root, "cancelled.apparent")]), {
      ...options, signal: cancelRefresh.signal,
    }), { ok: false, error: { category: "cancelled", message: "cancelled" } });
    assert.deepEqual(await readFile(credentialFile), beforeCancellation); assert.equal(metadata, beforeMetadata);
  } finally { server.closeAllConnections(); server.close(); await once(server, "close"); }
}));

test("Darwin ACL allow entries refuse publication and deny-only entries remain accepted", { skip: process.platform !== "darwin" }, async () => fixture(async root => {
  const { execFileSync } = await import("node:child_process");
  const destination = path.join(root, "acl.apparent");
  try {
    execFileSync("/bin/chmod", ["+a", "everyone allow read", root]);
    await assert.rejects(() => publishRecipientCapture(capture(), destination, resources), isCategory("unsafe_directory"));
    assert.deepEqual((await readdir(root)).sort(), []);
    execFileSync("/bin/chmod", ["-N", root]);
    execFileSync("/bin/chmod", ["+a", "everyone deny delete", root]);
    assert.deepEqual(await resources.checkAcl(await import("node:fs/promises").then(fs => fs.open(root, constants.O_RDONLY | constants.O_DIRECTORY))), { state: "accepted" });
    assert.deepEqual(await publishRecipientCapture(capture(), destination, resources), { state: "committed", path: destination });
  } finally { execFileSync("/bin/chmod", ["-N", root]); }
}));


test("native credential reader consumes a private descriptor and preserves stored authorization facts", async () => fixture(async root => {
  const file = path.join(root, "read.credential");
  const server = "https://app.tryapparent.com/connect";
  await writeFile(file, JSON.stringify({ server, scope: "read", token: "synthetic-private-token", generation: "b".repeat(32), authRequired: true }), { mode: 0o600 });
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  const credential = await readStoredCredentialHandle(handle, { scope: "read", approvedServer: server }, resources);
  assert.equal(credential.token, "synthetic-private-token"); assert.equal(credential.authRequired, true);
  await assert.rejects(handle.stat());
}));

test("native credential reader rejects Darwin file ACL grants before returning stored secrets", { skip: process.platform !== "darwin" }, async () => fixture(async root => {
  const { execFileSync } = await import("node:child_process");
  const file = path.join(root, "read.credential"), server = "https://app.tryapparent.com/connect";
  await writeFile(file, JSON.stringify({ server, scope: "read", token: "synthetic-private-token", generation: "b".repeat(32) }), { mode: 0o600 });
  try {
    execFileSync("/bin/chmod", ["+a", "everyone allow read", file]);
    const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    await assert.rejects(readStoredCredentialHandle(handle, { scope: "read", approvedServer: server }, resources), error => error instanceof CredentialReadError && error.category === "unsafe_file");
    await assert.rejects(handle.stat());
  } finally { execFileSync("/bin/chmod", ["-N", file]); }
}));


test("native state snapshots use the published lock lease and preserve the stable lock inode", async () => fixture(async root => {
  const state = path.join(root, "state"); await mkdir(state, { mode: 0o700 });
  const server = "https://app.tryapparent.com/connect";
  await writeFile(path.join(state, "read.credential"), JSON.stringify({ server, scope: "read", token: "synthetic-private-token", generation: "c".repeat(32) }), { mode: 0o600 });
  const snapshot = await readCredentialSnapshot(state, { scope: "read", approvedServer: server }, resources);
  const inode = (await lstat(path.join(state, "queue.lock"))).ino;
  await assertCredentialSnapshotCurrent(state, snapshot, resources);
  assert.equal((await lstat(path.join(state, "queue.lock"))).ino, inode);
  assert.deepEqual((await readdir(state)).sort(), ["queue.lock", "read.credential"]);
}));


test("native snapshot reads obey real flock contention and fresh cancellation/revocation fences", async () => fixture(async root => {
  const state = path.join(root, "state"); await mkdir(state, { mode: 0o700 });
  const server = "https://app.tryapparent.com/connect", context = { scope: "read" as const, approvedServer: server };
  await writeFile(path.join(state, "read.credential"), JSON.stringify({ server, scope: "read", token: "synthetic-private-token", generation: "d".repeat(32) }), { mode: 0o600 });
  const snapshot = await readCredentialSnapshot(state, context, resources);
  const held = await resources.lock(await open(path.join(state, "queue.lock"), constants.O_RDWR | constants.O_NOFOLLOW), { timeoutMs: 0 });
  assert.equal(held.state, "acquired"); if (held.state !== "acquired") throw new Error("missing_lease");
  try { await assert.rejects(readCredentialSnapshot(state, context, resources, { lockTimeoutMs: 15 }), error => error instanceof CredentialSnapshotError && error.category === "lock_refused"); }
  finally { await held.lease.release(); }
  await writeFile(path.join(state, "cancellation"), JSON.stringify("new epoch"), { mode: 0o600 });
  await assert.rejects(assertCredentialSnapshotCurrent(state, snapshot, resources), error => error instanceof CredentialSnapshotError && error.category === "stale_snapshot");
  await writeFile(path.join(state, "revocation-synthetic.intent"), "false", { mode: 0o600 });
  await assert.rejects(readCredentialSnapshot(state, context, resources), error => error instanceof CredentialSnapshotError && error.category === "revocation_pending");
}));


test("native snapshots refuse Darwin cancellation-record ACL grants", { skip: process.platform !== "darwin" }, async () => fixture(async root => {
  const { execFileSync } = await import("node:child_process");
  const state = path.join(root, "state"), server = "https://app.tryapparent.com/connect";
  await mkdir(state, { mode: 0o700 });
  await writeFile(path.join(state, "read.credential"), JSON.stringify({ server, scope: "read", token: "synthetic-private-token", generation: "e".repeat(32) }), { mode: 0o600 });
  const cancellation = path.join(state, "cancellation");
  await writeFile(cancellation, JSON.stringify("epoch"), { mode: 0o600 });
  try {
    execFileSync("/bin/chmod", ["+a", "everyone allow read", cancellation]);
    await assert.rejects(readCredentialSnapshot(state, { scope: "read", approvedServer: server }, resources), error => error instanceof CredentialSnapshotError && error.category === "unsafe_file");
  } finally { execFileSync("/bin/chmod", ["-N", cancellation]); }
}));

test("native private-state replacement commits bounded bytes under the real queue lease", async () => fixture(async root => {
  const state = path.join(root, "synthetic-state"); await mkdir(state, { mode: 0o700 });
  const lock = await open(path.join(state, "queue.lock"), constants.O_RDWR | constants.O_CREAT | constants.O_NOFOLLOW, 0o600);
  const acquired = await resources.lock(lock, { timeoutMs: 0 });
  assert.equal(acquired.state, "acquired"); if (acquired.state !== "acquired") return;
  try {
    const destination = path.join(state, "synthetic.test");
    const input = Buffer.from('{"version":1,"synthetic":true}');
    assert.deepEqual(await replacePrivateStateFile(state, "synthetic.test", input, null, acquired.lease, resources), { state: "committed" });
    assert.equal((await readFile(destination)).toString(), input.toString());
    const previous = await lstat(destination, { bigint: true });
    const expected = { device: String(previous.dev), inode: String(previous.ino) };
    const next = Buffer.from('{"version":1,"synthetic":"replacement"}');
    assert.deepEqual(await replacePrivateStateFile(state, "synthetic.test", next, expected, acquired.lease, resources), { state: "committed" });
    assert.equal((await readFile(destination)).toString(), next.toString());
    assert.equal((await lstat(destination)).mode & 0o777, 0o600);
    assert.deepEqual((await readdir(state)).sort(), ["queue.lock", "synthetic.test"]);
  } finally { await acquired.lease.release(); }
}));

test("native stale replacement refuses effect and removes only its private stage", async () => fixture(async root => {
  const destination = path.join(root, "synthetic.test"); await writeFile(destination, "old", { mode: 0o600 });
  const previous = await lstat(destination, { bigint: true });
  const lock = await open(path.join(root, "queue.lock"), constants.O_RDWR | constants.O_CREAT | constants.O_NOFOLLOW, 0o600);
  const acquired = await resources.lock(lock, { timeoutMs: 0 });
  assert.equal(acquired.state, "acquired"); if (acquired.state !== "acquired") return;
  try {
    const expected = { device: String(previous.dev), inode: String(previous.ino + 1n) };
    assert.deepEqual(await replacePrivateStateFile(root, "synthetic.test", Buffer.from("new"), expected, acquired.lease, resources), { state: "destination_changed" });
    assert.equal(await readFile(destination, "utf8"), "old");
    assert.deepEqual((await readdir(root)).sort(), ["queue.lock", "synthetic.test"]);
  } finally { await acquired.lease.release(); }
}));

test("installed provider cannot retire during Connect calls or an owned queue lease", { skip: !installedResources }, async () => fixture(async root => {
  const handle = await open(root, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
  const pending = resources.checkAcl(handle);
  await assert.rejects(installedResources!.close(), error => !!error && typeof error === "object" && "category" in error && error.category === "invalid_request");
  assert.deepEqual(await pending, { state: "accepted" });
  assert.equal(handle.fd, -1);
  const lock = await open(path.join(root, "queue.lock"), constants.O_CREAT | constants.O_RDWR | constants.O_NOFOLLOW, 0o600);
  const acquired = await resources.lock(lock, { timeoutMs: 0 });
  assert.equal(acquired.state, "acquired"); if (acquired.state !== "acquired") return;
  try {
    await assert.rejects(installedResources!.close(), error => !!error && typeof error === "object" && "category" in error && error.category === "invalid_request");
    const input = Buffer.from("synthetic lifetime proof");
    assert.deepEqual(await replacePrivateStateFile(root, "synthetic.test", input, null, acquired.lease, resources), { state: "committed" });
    assert.deepEqual(await readFile(path.join(root, "synthetic.test")), input);
  } finally { await acquired.lease.release(); }
}));

async function completedRenewalState(root: string, scope: "read" | "upload" = "read") {
  const state = path.join(root, "synthetic-state"); await mkdir(state, { mode: 0o700 });
  const context = { scope, approvedServer: "https://sync.example/connect" };
  const base = { server: context.approvedServer, scope, token: "synthetic-old-access", generation: "c".repeat(32),
    refreshToken: "synthetic-old-refresh", expiresAt: 1900, refreshExpiresAt: 3000, grantId: "synthetic-grant" };
  const put = (name: string, value: unknown) => writeFile(path.join(state, name), JSON.stringify(value), { mode: 0o600 });
  await put(`${scope}.credential`, base); await put(`${scope}.renewal`, { base, issued: 2000, completed: true });
  await put(`${scope}.response`, { scope, accessToken: "synthetic-new-access", refreshToken: "synthetic-new-refresh", grantId: "synthetic-grant", expiresIn: 60, refreshExpiresIn: 300 });
  return { state, context, snapshot: await readCredentialSnapshot(state, context, resources), put };
}

test("native completed renewal recovery commits both scopes and durably clears only their evidence", async () => {
  for (const scope of ["read", "upload"] as const) await fixture(async root => {
    const f = await completedRenewalState(root, scope), other = scope === "read" ? "upload" : "read";
    await f.put(`${other}.credential`, { unrelated: true });
    const result = await commitCompletedRenewal(f.state, f.context, f.snapshot, resources);
    assert.equal(result.state, "committed");
    const stored = JSON.parse(await readFile(path.join(f.state, `${scope}.credential`), "utf8"));
    assert.equal(stored.token, "synthetic-new-access"); assert.equal(stored.expiresAt, 2060); assert.equal(stored.refreshExpiresAt, 2300);
    assert.equal((await lstat(path.join(f.state, `${scope}.credential`))).mode & 0o777, 0o600);
    assert.deepEqual(JSON.parse(await readFile(path.join(f.state, `${other}.credential`), "utf8")), { unrelated: true });
    assert.deepEqual((await readdir(f.state)).sort(), ["queue.lock", `${scope}.credential`, `${scope}.renewal.lock`, `${other}.credential`].sort());
  });
});

test("native scope contention refuses recovery before queue mutation and preserves the lock inode", async () => fixture(async root => {
  const f = await completedRenewalState(root);
  const scopeFile = path.join(f.state, "read.renewal.lock");
  const acquired = await resources.lock(await open(scopeFile, constants.O_CREAT | constants.O_RDWR | constants.O_NOFOLLOW, 0o600), { timeoutMs: 0 });
  assert.equal(acquired.state, "acquired"); if (acquired.state !== "acquired") return;
  const original = await lstat(scopeFile);
  try {
    assert.deepEqual(await commitCompletedRenewal(f.state, f.context, f.snapshot, resources, { lockTimeoutMs: 0 }), { state: "unchanged", reason: "lock_refused" });
    assert.equal(JSON.parse(await readFile(path.join(f.state, "read.credential"), "utf8")).token, "synthetic-old-access");
    assert.ok((await readdir(f.state)).includes("read.response"));
  } finally { await acquired.lease.release(); }
  assert.equal((await commitCompletedRenewal(f.state, f.context, f.snapshot, resources)).state, "committed");
  assert.equal((await lstat(scopeFile)).ino, original.ino);
}));

test("native recovery retains substituted evidence after a verified credential effect", async () => fixture(async root => {
  const f = await completedRenewalState(root); let calls = 0;
  const observed: NamespaceResources & StateReplacementResources = {
    checkAcl: handle => resources.checkAcl(handle), lock: (handle, options) => resources.lock(handle, options),
    async replaceFile(...args) {
      calls++; const result = await resources.replaceFile(...args);
      if (result.state === "replaced") {
        await rename(path.join(f.state, "read.renewal"), path.join(f.state, "original.renewal"));
        await f.put("read.renewal", { substituted: true });
      }
      return result;
    },
  };
  assert.deepEqual(await commitCompletedRenewal(f.state, f.context, f.snapshot, observed), { state: "publication_uncertain" });
  assert.equal(JSON.parse(await readFile(path.join(f.state, "read.credential"), "utf8")).token, "synthetic-new-access");
  assert.deepEqual(JSON.parse(await readFile(path.join(f.state, "read.renewal"), "utf8")), { substituted: true });
  assert.ok((await readdir(f.state)).includes("read.response")); assert.equal(calls, 1);
}));

test("native renewal exchange persists both scopes through real HTTP before recovery", async () => {
  for (const scope of ["read", "upload"] as const) await fixture(async root => {
    let requests = 0;
    const server = createServer(async (req, res) => {
      try {
        requests++; assert.equal(req.url, "/connect/v1/token/refresh"); assert.equal(req.method, "POST");
        assert.equal(req.headers.authorization, undefined); assert.equal(req.headers.cookie, undefined);
        const chunks: Buffer[] = []; for await (const chunk of req) chunks.push(Buffer.from(chunk));
        assert.deepEqual(JSON.parse(Buffer.concat(chunks).toString()), { refreshToken: "synthetic-refresh" });
        const queued = await resources.lock(await open(path.join(root, "queue.lock"), constants.O_RDWR | constants.O_NOFOLLOW), { timeoutMs: 0 });
        assert.equal(queued.state, "acquired"); if (queued.state === "acquired") await queued.lease.release();
        const scoped = await resources.lock(await open(path.join(root, `${scope}.renewal.lock`), constants.O_RDWR | constants.O_NOFOLLOW), { timeoutMs: 0 });
        if (scoped.state === "acquired") await scoped.lease.release(); assert.equal(scoped.state, "busy");
        const journal = JSON.parse(await readFile(path.join(root, `${scope}.renewal`), "utf8"));
        assert.equal(journal.completed, false); assert.equal(journal.issued, 2000);
        res.setHeader("Content-Type", "application/json");
        res.end(JSON.stringify({ scope, accessToken: "synthetic-new-access", refreshToken: "synthetic-new-refresh", grantId: "synthetic-grant", expiresIn: 60, refreshExpiresIn: 300 }));
      } catch { res.statusCode = 500; res.end(); }
    });
    server.listen(0, "127.0.0.1"); await once(server, "listening");
    try {
      const address = server.address(); assert.ok(address && typeof address === "object");
      const context = { scope, approvedServer: `http://127.0.0.1:${address.port}/connect` };
      const original = { server: context.approvedServer, scope, token: "synthetic-access", generation: "e".repeat(32),
        refreshToken: "synthetic-refresh", expiresAt: 1900, refreshExpiresAt: 3000, grantId: "synthetic-grant" };
      await writeFile(path.join(root, `${scope}.credential`), JSON.stringify(original), { mode: 0o600 });
      const snapshot = await readCredentialSnapshot(root, context, resources);
      assert.deepEqual(await persistRenewalExchange(root, context, snapshot, resources, { now: () => 2000 }), { state: "completed" });
      assert.equal(requests, 1); assert.deepEqual(JSON.parse(await readFile(path.join(root, `${scope}.credential`), "utf8")), original);
      assert.equal((await lstat(path.join(root, `${scope}.response`))).mode & 0o777, 0o600);
      assert.equal(JSON.parse(await readFile(path.join(root, `${scope}.renewal`), "utf8")).completed, true);
      assert.equal((await commitCompletedRenewal(root, context, snapshot, resources)).state, "committed");
      const current = JSON.parse(await readFile(path.join(root, `${scope}.credential`), "utf8"));
      assert.equal(current.expiresAt, 2060); assert.equal(current.token, "synthetic-new-access");
      assert.deepEqual((await readdir(root)).sort(), ["queue.lock", `${scope}.credential`, `${scope}.renewal.lock`].sort());
    } finally { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
  });
});

test("native renewal exchange fences a cooperating cancellation during real HTTP", async () => fixture(async root => {
  let requests = 0;
  const server = createServer(async (_req, res) => {
    requests++;
    const acquired = await resources.lock(await open(path.join(root, "queue.lock"), constants.O_RDWR | constants.O_NOFOLLOW), { timeoutMs: 1000 });
    if (acquired.state !== "acquired") { res.statusCode = 500; res.end(); return; }
    try { await writeFile(path.join(root, "cancellation"), JSON.stringify("new-epoch"), { mode: 0o600 }); }
    finally { await acquired.lease.release(); }
    res.end(JSON.stringify({ scope: "read", accessToken: "new", refreshToken: "rotated", grantId: "grant", expiresIn: 60, refreshExpiresIn: 300 }));
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  try {
    const address = server.address(); assert.ok(address && typeof address === "object");
    const context = { scope: "read" as const, approvedServer: `http://127.0.0.1:${address.port}/connect` };
    const base = { server: context.approvedServer, scope: "read", token: "old", generation: "f".repeat(32), refreshToken: "refresh", expiresAt: 1900, refreshExpiresAt: 3000, grantId: "grant" };
    await writeFile(path.join(root, "read.credential"), JSON.stringify(base), { mode: 0o600 });
    const snapshot = await readCredentialSnapshot(root, context, resources);
    assert.deepEqual(await persistRenewalExchange(root, context, snapshot, resources, { now: () => 2000 }), { state: "stopped", phase: "request_sent", reason: "stale_snapshot" });
    assert.equal(requests, 1); await assert.rejects(lstat(path.join(root, "read.response")), { code: "ENOENT" });
    assert.deepEqual(JSON.parse(await readFile(path.join(root, "read.credential"), "utf8")), base);
    assert.equal(JSON.parse(await readFile(path.join(root, "read.renewal"), "utf8")).completed, false);
  } finally { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
}));

test("native renewal coordinator joins both scopes and terminal rejection through real HTTP", async () => {
  for (const scope of ["read", "upload"] as const) for (const response of ["rotate", "reject", "invalid"] as const) await fixture(async root => {
    let requests = 0;
    const server = createServer(async (req, res) => {
      requests++;
      for await (const _chunk of req) { /* Drain the bounded synthetic request. */ }
      if (req.url !== "/connect/v1/token/refresh" || req.method !== "POST" || req.headers.authorization || req.headers.cookie) {
        res.statusCode = 400; res.end(); return;
      }
      if (response === "reject") { res.statusCode = 401; res.end(); }
      else if (response === "invalid") res.end("not JSON");
      else res.end(JSON.stringify({ scope, accessToken: "new-access", refreshToken: "new-refresh", grantId: "grant", expiresIn: 60, refreshExpiresIn: 300 }));
    });
    server.listen(0, "127.0.0.1"); await once(server, "listening");
    try {
      const address = server.address(); assert.ok(address && typeof address === "object");
      const context = { scope, approvedServer: `http://127.0.0.1:${address.port}/connect` };
      const base = { server: context.approvedServer, scope, token: "old-access", generation: "a".repeat(32), refreshToken: "old-refresh", expiresAt: 1900, refreshExpiresAt: 3000, grantId: "grant" };
      const other = scope === "read" ? "upload" : "read";
      await writeFile(path.join(root, `${scope}.credential`), JSON.stringify(base), { mode: 0o600 });
      await writeFile(path.join(root, `${other}.credential`), '{"retained":true}', { mode: 0o600 });
      await writeFile(path.join(root, "config"), '{"retained":true}', { mode: 0o600 });
      const snapshot = await readCredentialSnapshot(root, context, resources);
      const result = await renewStateCredential(root, context, snapshot, resources, { now: () => 2000 });
      assert.equal(requests, 1);
      const stored = JSON.parse(await readFile(path.join(root, `${scope}.credential`), "utf8"));
      if (response === "rotate") {
        assert.equal(result.state, "credential"); if (result.state === "credential") assert.equal(result.source, "renewed");
        assert.equal(stored.token, "new-access"); assert.equal(stored.expiresAt, 2060);
        await assert.rejects(lstat(path.join(root, `${scope}.renewal`)), { code: "ENOENT" });
        await assert.rejects(lstat(path.join(root, `${scope}.response`)), { code: "ENOENT" });
      } else {
        assert.deepEqual(result, { state: "auth_required" }); assert.equal(stored.authRequired, true); assert.equal(stored.token, base.token);
        assert.equal(stored.refreshToken, base.refreshToken); assert.equal(stored.generation, base.generation);
        assert.equal(JSON.parse(await readFile(path.join(root, `${scope}.renewal`), "utf8")).completed, response === "invalid");
        if (response === "invalid") assert.equal(await readFile(path.join(root, `${scope}.response`), "utf8"), "not JSON");
      }
      assert.equal((await lstat(path.join(root, `${scope}.credential`))).mode & 0o777, 0o600);
      assert.equal(await readFile(path.join(root, `${other}.credential`), "utf8"), '{"retained":true}');
      assert.equal(await readFile(path.join(root, "config"), "utf8"), '{"retained":true}');
    } finally { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
  });
});

test("native renewal coordinator reloads a real cooperating recovery winner between phases", async () => fixture(async root => {
  const f = await completedRenewalState(root); let recovered = false;
  const observed: NamespaceResources & StateReplacementResources = {
    checkAcl: handle => resources.checkAcl(handle), replaceFile: (...args) => resources.replaceFile(...args),
    async lock(handle, options) {
      const inode = (await handle.stat()).ino;
      const isScope = inode === (await lstat(path.join(f.state, "read.renewal.lock"))).ino;
      const result = await resources.lock(handle, options);
      if (result.state !== "acquired" || !isScope || recovered) return result;
      return { state: "acquired", lease: { signal: result.lease.signal, assertHeld: () => result.lease.assertHeld(), async release() {
        await result.lease.release(); recovered = true;
        assert.equal((await commitCompletedRenewal(f.state, f.context, f.snapshot, resources)).state, "committed");
      } } };
    },
  };
  const result = await renewStateCredential(f.state, f.context, f.snapshot, observed, { now: () => 2000, request: async () => { throw new Error("unexpected POST"); } });
  assert.equal(recovered, true); assert.equal(result.state, "credential");
  if (result.state === "credential") { assert.equal(result.source, "other_writer"); assert.equal(result.credential.token, "synthetic-new-access"); }
  await assert.rejects(lstat(path.join(f.state, "read.response")), { code: "ENOENT" });
  assert.equal(JSON.parse(await readFile(path.join(f.state, "read.credential"), "utf8")).expiresAt, 2060);
}));

function writerFixtureOwner(writer: string, config: string): RecipientRenewalOwner {
  return async (request, signal) => {
    const child = spawn(writer, [], { env: { ...process.env, APPARENT_CONFIG: config }, stdio: ["pipe", "pipe", "pipe"] });
    const output: Buffer[] = []; let size = 0;
    child.stdout.on("data", (chunk: Buffer) => { size += chunk.length; if (size > 4096) child.kill("SIGKILL"); else output.push(chunk); });
    child.stderr.resume();
    const cancel = () => child.kill("SIGTERM"); signal?.addEventListener("abort", cancel, { once: true });
    const timer = setTimeout(() => child.kill("SIGKILL"), 35_000);
    try {
      if (signal?.aborted) cancel();
      child.stdin.on("error", () => {}); child.stdin.end(JSON.stringify(request));
      const [code] = await once(child, "close");
      if (size > 4096) return { state: "state_unavailable" };
      return decodeRecipientRenewalReply(Buffer.concat(output), typeof code === "number" ? code : -1, request.operation.kind);
    } finally { clearTimeout(timer); signal?.removeEventListener("abort", cancel); }
  };
}

test("recipient host joins admitted installed resources with the existing Rust writer fixture", {
  skip: !installedResources || !process.env.APPARENT_RECIPIENT_WRITER_BIN,
}, async () => fixture(async root => {
  const writer = absoluteEnv("APPARENT_RECIPIENT_WRITER_BIN", process.env.APPARENT_RECIPIENT_WRITER_BIN);
  assert.equal(digest(await readFile(writer)), process.env.APPARENT_RECIPIENT_WRITER_SHA256);
  const state = path.join(root, "hosted"), config = path.join(root, "config.json"), id = "2".repeat(32);
  const configBytes = '{"unknown":"retained","cloud":{"enabled":false,"uploadConsent":false}}', uploadBytes = '{"unrelated":"retained"}';
  await mkdir(state, { mode: 0o700 }); await writeFile(config, configBytes, { mode: 0o600 });
  await writeFile(path.join(state, "upload.credential"), uploadBytes, { mode: 0o600 });
  const assets = new Map([["summary.md", Buffer.from("synthetic host inspection\n")], ["apparent.json", Buffer.from("{}")], ["contact-sheet.png", Buffer.from([1, 2, 3])]]);
  const files = [...assets].map(([path, bytes]) => ({ path, size: bytes.length, sha256: digest(bytes) }));
  let refreshes = 0, providers = 0, inspection = 0;
  let inspected: { json: boolean; summary: string; evidenceMarker: boolean } | undefined;
  let inspectionError: unknown;
  const server = createServer(async (request, response) => {
    const route = request.url?.replace(/^\/connect(?=\/)/, "");
    if (route === "/v1/token/refresh") {
      assert.equal(request.method, "POST"); assert.equal(request.headers.authorization, undefined);
      let body = ""; for await (const chunk of request) body += chunk;
      assert.deepEqual(JSON.parse(body), { refreshToken: "synthetic-refresh" }); refreshes++;
      response.end(JSON.stringify({ scope: "read", grantId: "read-grant", accessToken: "synthetic-new-access", refreshToken: "synthetic-new-refresh", expiresIn: 3600, refreshExpiresIn: 7200 })); return;
    }
    assert.equal(request.headers.authorization, "Bearer synthetic-new-access"); assert.equal(request.headers.cookie, undefined);
    if (route === `/v1/captures/${id}`) response.end(JSON.stringify({ id, status: "ready", files }));
    else if (route?.startsWith(`/v1/captures/${id}/files/`) && assets.has(route.slice(`/v1/captures/${id}/files/`.length))) response.end(assets.get(route.slice(`/v1/captures/${id}/files/`.length)));
    else response.writeHead(404).end();
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  try {
    const address = server.address(); assert.ok(address && typeof address === "object");
    const origin = `http://127.0.0.1:${address.port}/connect`;
    await writeFile(path.join(state, "read.credential"), JSON.stringify({ server: origin, scope: "read", token: "synthetic-old-access", generation: "b".repeat(32),
      refreshToken: "synthetic-refresh", grantId: "read-grant", expiresAt: 1, refreshExpiresAt: Math.floor(Date.now() / 1000) + 7200 }), { mode: 0o600 });
    const result = await runRecipientHost(["inspect", id, "--json"], {
      cwd: root,
      resolveRemote: async () => ({ directory: state, approvedServer: origin, allowLegacyPlaintext: true,
        admission: { artifactDirectory: artifact, trustedManifestSha256: trustedManifestSha256!, expectedSourceSha: expectedSource!, target: target as "linux-x64" | "macos-x64" },
        writer: { directory: state, approvedServer: origin, renewRead: writerFixtureOwner(writer, config) } }),
      async openResources(admission) { providers++; return nativeModule.createInstalledNativeResources(admission); },
      async inspect(source, options) {
        inspection++;
        try {
          const marker = await lstat(path.join(source, ".evidence-only.json")).catch(error => {
            if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error;
          });
          inspected = { json: options.json, summary: await readFile(path.join(source, "summary.md"), "utf8"), evidenceMarker: marker !== null };
        } catch (error) { inspectionError = error; throw error; }
        return { stdout: "synthetic Agent output\n", stderr: "" };
      },
    });
    assert.equal(result.receipt.exitCode, 0, JSON.stringify({ receipt: result.receipt, inspectionError: String(inspectionError) })); assert.equal(result.receipt.stdout, "synthetic Agent output\n");
    assert.equal(result.provider.state, "retired"); assert.equal(result.receipt.publication?.state, "committed");
    assert.equal(refreshes, 1); assert.equal(providers, 1); assert.equal(inspection, 1);
    assert.deepEqual(inspected, { json: true, summary: "synthetic host inspection\n", evidenceMarker: false });
    assert.equal(await readFile(config, "utf8"), configBytes); assert.equal(await readFile(path.join(state, "upload.credential"), "utf8"), uploadBytes);
    assert.equal(JSON.parse(await readFile(path.join(state, "read.credential"), "utf8")).token, "synthetic-new-access");
  } finally { server.closeAllConnections(); await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
}));

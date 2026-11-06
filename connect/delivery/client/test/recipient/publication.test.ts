import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, lstat, readdir, realpath, rm, rename, symlink, chmod } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { publishRecipientCapture, RecipientPublicationError, type RecipientPublicationResources } from "../../src/recipient/publication.ts";
import type { RecipientCliDependencies } from "../../src/cli/recipient.ts";

const hash = (bytes: Uint8Array) => createHash(bytes);
function createHash(bytes: Uint8Array) {
  // WebCrypto keeps the fixture independent of authored JS helpers.
  return Array.from(new Uint8Array(requireCrypto().digest("sha256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
}
function requireCrypto(): { digest(algorithm: string, bytes: Uint8Array): Uint8Array } {
  // Node's built-in synchronous hash is imported dynamically by the test runner.
  return { digest: (algorithm, bytes) => {
    const h = createHashNode(algorithm); h.update(bytes); return h.digest();
  } };
}
import { createHash as createHashNode } from "node:crypto";

function capture(evidenceOnly = false) {
  const entries = [
    ["summary.md", Buffer.from("a synthetic summary")],
    ["contact-sheet.png", Buffer.from([1, 2, 3])],
    ["apparent.json", Buffer.from(JSON.stringify({ contactSheet: "contact-sheet.png", keyframes: [{ file: "frames/frame-01.png" }] }))],
    ["frames/frame-01.png", Buffer.from([4, 5, 6])],
  ] as const;
  return { captureId: "a".repeat(32), server: "https://connect.example", evidenceOnly,
    files: entries.map(([filePath, bytes]) => ({ path: filePath, bytes: new Uint8Array(bytes), size: bytes.length, sha256: hash(bytes) })) };
}
const publisherFitsCli: RecipientCliDependencies["publish"] = (value, destination) => publishRecipientCapture(value, destination, {
  checkAcl: async h => { await h.close(); return { state: "accepted" }; },
  publish: async h => { await h.close(); return { state: "publication_uncertain" }; },
});
void publisherFitsCli;
function localResources(parentPath: string, mode: "published" | "publication_uncertain" | "refused" = "published", onDispatch?: () => void): RecipientPublicationResources {
  return {
    checkAcl: async (handle) => { await handle.close(); return { state: "accepted" }; },
    publish: async (parent, source, destination) => {
      try {
        onDispatch?.();
        if (mode !== "published") return { state: mode };
        await rename(path.join(parentPath, source), path.join(parentPath, destination));
        return { state: "published" };
      } finally { await parent.close(); }
    },
  };
}
async function fixture<T>(fn: (root: string) => Promise<T>): Promise<T> {
  const root = await mkdtemp(path.join(await realpath(tmpdir()), "apparent-pub-"));
  try { return await fn(root); } finally { await rm(root, { recursive: true, force: true }); }
}

test("publishes a complete validated capture and evidence carries the exact legacy marker", async () => fixture(async root => {
  const out = path.join(root, "capture.apparent");
  const result = await publishRecipientCapture(capture(), out, localResources(root));
  assert.deepEqual(result, { state: "committed", path: out });
  assert.equal((await readFile(path.join(out, "frames/frame-01.png"))).length, 3);
  const evidenceOut = path.join(root, "evidence.apparent");
  const evidence = await publishRecipientCapture(capture(true), evidenceOut, localResources(root));
  assert.deepEqual(evidence, { state: "committed", path: evidenceOut, evidenceOnlyReceipt: true });
  assert.deepEqual(JSON.parse(await readFile(path.join(evidenceOut, ".evidence-only.json"), "utf8")), {
    version: 1, captureId: "a".repeat(32), server: "https://connect.example", media: "not downloaded",
    guidance: "Use apparent fetch <id> --media --out <new-directory> for the complete recording",
  });
}));

test("preserves Unicode and spaces in POSIX destination and asset names", async () => fixture(async root => {
  const value = capture(); value.files[0]!.path = "summary.md";
  const result = await publishRecipientCapture(value, path.join(root, "capture résumé 1.apparent"), localResources(root));
  assert.equal(result.state, "committed");
}));

test("rejects traversal, symlink parents and untrusted shared parents before dispatch", async () => fixture(async root => {
  let dispatched = false;
  await assert.rejects(() => publishRecipientCapture(capture(), `${root}/../bad`, localResources(root, "published", () => dispatched = true)),
    (e: unknown) => e instanceof RecipientPublicationError && e.category === "invalid_destination");
  const link = path.join(root, "link"); await symlink(root, link);
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(link, "bad"), localResources(root)), RecipientPublicationError);
  await chmod(root, 0o777);
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(root, "shared", "out"), localResources(root)), RecipientPublicationError);
  await chmod(root, 0o700);
  assert.equal(dispatched, false);
}));

test("existing destinations remain untouched and invalid bytes create no stage", async () => fixture(async root => {
  const out = path.join(root, "taken"); await mkdir(out); await (await import("node:fs/promises")).writeFile(path.join(out, "keep"), "original");
  await assert.rejects(() => publishRecipientCapture(capture(), out, localResources(root)), (e: unknown) => e instanceof RecipientPublicationError && e.category === "destination_exists");
  assert.equal(await readFile(path.join(out, "keep"), "utf8"), "original");
  const bad = capture(); bad.files[0]!.bytes[0] ^= 1;
  await assert.rejects(() => publishRecipientCapture(bad, path.join(root, "bad"), localResources(root)), RecipientPublicationError);
  const brokenEvidence = capture(true); brokenEvidence.files = brokenEvidence.files.filter(f => f.path !== "frames/frame-01.png");
  await assert.rejects(() => publishRecipientCapture(brokenEvidence, path.join(root, "broken-evidence"), localResources(root)), RecipientPublicationError);
  assert.deepEqual((await readdir(root)).sort(), ["taken"]);
}));

test("cancellation stops before staging; native uncertainty retains private staged evidence without retry", async () => fixture(async root => {
  const controller = new AbortController(); controller.abort();
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(root, "cancelled"), localResources(root), { signal: controller.signal }), RecipientPublicationError);
  let calls = 0;
  const uncertain = await publishRecipientCapture(capture(true), path.join(root, "maybe"), {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async parent => { calls++; await parent.close(); return { state: "publication_uncertain" }; },
  });
  assert.deepEqual(uncertain, { state: "publication-uncertain" }); assert.equal(calls, 1);
  const stages = (await readdir(root)).filter(n => n.startsWith(".apparent-stage-")); assert.equal(stages.length, 1);
  const stage = path.join(root, stages[0]!);
  assert.equal((await lstat(stage)).mode & 0o777, 0o700);
  assert.equal((await lstat(path.join(stage, "summary.md"))).mode & 0o777, 0o600);
  assert.ok(await readFile(path.join(stage, ".evidence-only.json")));
}));

test("a parent path replaced during native dispatch never reports committed", async () => fixture(async root => {
  const parentPath = path.join(root, "out"); await mkdir(parentPath, { mode: 0o700 });
  const result = await publishRecipientCapture(capture(), path.join(parentPath, "capture"), {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async (parent, source, destination) => {
      try {
        await rename(parentPath, path.join(root, "moved")); await mkdir(parentPath);
        await rename(path.join(root, "moved", source), path.join(root, "moved", destination));
        return { state: "published" };
      } finally { await parent.close(); }
    },
  });
  assert.deepEqual(result, { state: "publication-uncertain" });
  assert.ok(await readFile(path.join(root, "moved", "capture", "summary.md")));
}));

test("a native published result without matching destination readback is uncertain", async () => fixture(async root => {
  let calls = 0;
  const result = await publishRecipientCapture(capture(), path.join(root, "missing-readback"), {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async parent => { calls++; await parent.close(); return { state: "published" }; },
  });
  assert.deepEqual(result, { state: "publication-uncertain" }); assert.equal(calls, 1);
  assert.equal((await readdir(root)).filter(n => n.startsWith(".apparent-stage-")).length, 1);
}));

test("typed exceptions after native dispatch are uncertain and preserve committed output", async () => fixture(async root => {
  let calls = 0;
  const out = path.join(root, "published-then-threw");
  const result = await publishRecipientCapture(capture(), out, {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async (parent, source, destination) => {
      calls++;
      try { await rename(path.join(root, source), path.join(root, destination)); throw new RecipientPublicationError("cancelled"); }
      finally { await parent.close(); }
    },
  });
  assert.deepEqual(result, { state: "publication-uncertain" });
  assert.equal(calls, 1);
  assert.equal(await readFile(path.join(out, "summary.md"), "utf8"), "a synthetic summary");
}));

test("known refusal removes only the unchanged stage; replaced parent retains foreign data and stage", async () => fixture(async root => {
  const parentPath = path.join(root, "out"); await mkdir(parentPath, { mode: 0o700 });
  const result = await publishRecipientCapture(capture(), path.join(parentPath, "out.apparent"), {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async (parent, source) => {
      try {
        await rename(parentPath, path.join(root, "moved")); await mkdir(parentPath, { mode: 0o700 });
        await (await import("node:fs/promises")).writeFile(path.join(parentPath, "foreign-sentinel"), "keep");
        return { state: "refused" };
      } finally { await parent.close(); }
    },
  });
  assert.deepEqual(result, { state: "publication-uncertain" });
  assert.equal(await readFile(path.join(parentPath, "foreign-sentinel"), "utf8"), "keep");
  assert.equal((await readdir(path.join(root, "moved"))).filter(n => n.startsWith(".apparent-stage-")).length, 1);
}));

test("cancellation after staging removes its unchanged private stage", async () => fixture(async root => {
  const controller = new AbortController();
  let checked = 0;
  const resources: RecipientPublicationResources = {
    checkAcl: async h => { checked++; await h.close(); if (checked > 1) controller.abort(); return { state: "accepted" }; },
    publish: async h => { await h.close(); return { state: "published" }; },
  };
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(root, "cancel-stage"), resources, { signal: controller.signal }), RecipientPublicationError);
  assert.deepEqual(await readdir(root), []);
}));

test("known native refusal removes its unchanged stage", async () => fixture(async root => {
  await assert.rejects(() => publishRecipientCapture(capture(), path.join(root, "out"), localResources(root, "refused")),
    (e: unknown) => e instanceof RecipientPublicationError && e.category === "publication_refused");
  assert.deepEqual(await readdir(root), []);
}));

test("receipt uses the validated capture snapshot after caller mutation", async () => fixture(async root => {
  const value = capture(true); let mutated = false;
  const result = await publishRecipientCapture(value, path.join(root, "evidence"), {
    checkAcl: async h => {
      await h.close();
      if (!mutated) { mutated = true; value.captureId = "b".repeat(32); value.server = "https://changed.invalid"; value.evidenceOnly = false; }
      return { state: "accepted" };
    },
    publish: async (parent, source, destination) => { try { await rename(path.join(root, source), path.join(root, destination)); return { state: "published" }; } finally { await parent.close(); } },
  });
  assert.deepEqual(result, { state: "committed", path: path.join(root, "evidence"), evidenceOnlyReceipt: true });
  assert.deepEqual(JSON.parse(await readFile(path.join(root, "evidence", ".evidence-only.json"), "utf8")), {
    version: 1, captureId: "a".repeat(32), server: "https://connect.example", media: "not downloaded",
    guidance: "Use apparent fetch <id> --media --out <new-directory> for the complete recording",
  });
}));

test("replaced staged asset directory preserves foreign sentinel and uncertain stage", async () => fixture(async root => {
  const foreign = path.join(root, "foreign"); await mkdir(foreign);
  await (await import("node:fs/promises")).writeFile(path.join(foreign, "sentinel"), "keep");
  const result = await publishRecipientCapture(capture(), path.join(root, "out"), {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async (parent, source) => {
      try {
        const stage = path.join(root, source);
        await rename(path.join(stage, "frames"), path.join(root, "displaced-frames"));
        await symlink(foreign, path.join(stage, "frames"));
        return { state: "refused" };
      } finally { await parent.close(); }
    },
  });
  assert.deepEqual(result, { state: "publication-uncertain" });
  assert.equal(await readFile(path.join(foreign, "sentinel"), "utf8"), "keep");
  const stage = (await readdir(root)).find(n => n.startsWith(".apparent-stage-"));
  assert.ok(stage);
  assert.equal((await lstat(path.join(root, stage!, "frames"))).isSymbolicLink(), true);
}));


test("refusal cleanup preserves a replacement regular file", async () => fixture(async root => {
  const result = await publishRecipientCapture(capture(), path.join(root, "out"), {
    checkAcl: async h => { await h.close(); return { state: "accepted" }; },
    publish: async (parent, source) => {
      try {
        const summary = path.join(root, source, "summary.md");
        await rename(summary, path.join(root, "displaced-summary"));
        await (await import("node:fs/promises")).writeFile(summary, "foreign replacement", { mode: 0o600 });
        return { state: "refused" };
      } finally { await parent.close(); }
    },
  });
  assert.deepEqual(result, { state: "publication-uncertain" });
  const stage = (await readdir(root)).find(n => n.startsWith(".apparent-stage-"));
  assert.ok(stage);
  assert.equal(await readFile(path.join(root, stage, "summary.md"), "utf8"), "foreign replacement");
  assert.equal(await readFile(path.join(root, stage, "frames/frame-01.png")).then(b => b.length), 3);
}));

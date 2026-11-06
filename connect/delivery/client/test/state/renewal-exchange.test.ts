import test from "node:test";
import assert from "node:assert/strict";
import { chmod, lstat, mkdtemp, readFile, readdir, realpath, rename, rm, writeFile, type FileHandle } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { persistRenewalExchange, renewStateCredential } from "../../src/state/renewal-exchange.ts";
import { commitCompletedRenewal, markCredentialAuthRequired } from "../../src/state/renewal-commit.ts";
import { readCredentialSnapshot, type NamespaceResources } from "../../src/state/namespace.ts";
import type { StateReplacementResources } from "../../src/state/replacement.ts";
import { RenewalRequestError } from "../../src/credentials/renewal-request.ts";

const context = { scope: "read" as const, approvedServer: "https://sync.example/connect" };
const wire = { server: context.approvedServer, scope: "read", token: "old-access", generation: "a".repeat(32),
  refreshToken: "old-refresh", expiresAt: 1900, refreshExpiresAt: 3000, grantId: "grant" };
const reply = { scope: "read", accessToken: "new-access", refreshToken: "new-refresh", grantId: "grant", expiresIn: 60, refreshExpiresIn: 300 };
async function fixture(run: (f: Awaited<ReturnType<typeof createFixture>>) => Promise<void>) {
  const f = await createFixture(); try { await run(f); } finally { await rm(f.root, { recursive: true, force: true }); }
}
async function createFixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-renewal-exchange-")); await chmod(root, 0o700);
  const file = (name: string) => path.join(root, name);
  const put = (name: string, value: unknown) => writeFile(file(name), JSON.stringify(value), { mode: 0o600 });
  const json = async (name: string) => JSON.parse(await readFile(file(name), "utf8"));
  await put("read.credential", wire); await put("upload.credential", { retained: "upload" }); await put("config", { retained: true });
  const held = new Set<string>(), events: string[] = [];
  const state = { requests: 0, dispatches: 0, failDispatch: 0, refusedDispatch: 0, failRelease: "",
    hook: undefined as undefined | ((destination: string) => Promise<void>), beforeAcl: undefined as undefined | ((handle: FileHandle) => Promise<void>), afterScopeRelease: undefined as undefined | (() => Promise<void>) };
  const resources: NamespaceResources & StateReplacementResources = {
    async checkAcl(handle) { try { await state.beforeAcl?.(handle); return { state: "accepted" }; } finally { await handle.close(); } },
    async lock(handle) {
      const fd = await handle.stat(); let name = "";
      try { for (const candidate of await readdir(root)) if (candidate.endsWith(".lock") && (await lstat(file(candidate))).ino === fd.ino) name = candidate; }
      finally { await handle.close(); }
      assert.ok(name); assert.equal(held.has(name), false); held.add(name); events.push(`acquire:${name}`);
      return { state: "acquired", lease: { signal: new AbortController().signal, assertHeld() { assert.ok(held.has(name)); },
        async release() { held.delete(name); events.push(`release:${name}`); if (name === "read.renewal.lock" && state.afterScopeRelease) { const hook = state.afterScopeRelease; state.afterScopeRelease = undefined; await hook(); } if (state.failRelease === name) throw new Error("private diagnostic"); } } };
    },
    async replaceFile(parent, source, destination) {
      try {
        state.dispatches++; assert.ok(held.has("queue.lock")); if (destination !== "read.credential") assert.ok(held.has("read.renewal.lock"));
        events.push(`replace:${destination}`);
        if (state.refusedDispatch === state.dispatches) return { state: "refused" };
        await rename(file(source), file(destination)); await state.hook?.(destination);
        if (state.failDispatch === state.dispatches) throw new Error("lost private acknowledgment");
        return { state: "replaced" };
      } finally { await parent.close(); }
    },
  };
  const snapshot = await readCredentialSnapshot(root, context, resources); events.length = 0;
  const request = async () => {
    state.requests++; assert.deepEqual([...held], ["read.renewal.lock"]); events.push("POST");
    assert.equal((await json("read.renewal")).completed, false);
    await assert.rejects(readFile(file("read.response")), { code: "ENOENT" });
    return Buffer.from(JSON.stringify(reply));
  };
  const run = (options: Parameters<typeof persistRenewalExchange>[4] = {}) => persistRenewalExchange(root, context, snapshot, resources, { now: () => 2000, request, ...options });
  return { root, file, put, json, state, held, events, resources, snapshot, run, request };
}

test("journal precedes a single POST, raw response precedes completion, and recovery uses saved time", () => fixture(async f => {
  const result = await f.run(); assert.deepEqual(result, { state: "completed" });
  assert.equal(f.state.requests, 1); assert.equal(f.state.dispatches, 3); assert.equal(f.held.size, 0);
  assert.deepEqual(f.events, ["acquire:read.renewal.lock", "acquire:queue.lock", "replace:read.renewal", "release:queue.lock", "POST",
    "acquire:queue.lock", "replace:read.response", "replace:read.renewal", "release:queue.lock", "release:read.renewal.lock"]);
  assert.equal((await f.json("read.renewal")).issued, 2000); assert.equal((await f.json("read.renewal")).completed, true);
  assert.deepEqual(await f.json("read.credential"), wire); assert.deepEqual(await f.json("upload.credential"), { retained: "upload" });
  assert.deepEqual(await f.json("config"), { retained: true });
  assert.equal((await lstat(f.file("read.response"))).mode & 0o777, 0o600);
  const recovered = await commitCompletedRenewal(f.root, context, f.snapshot, f.resources);
  assert.equal(recovered.state, "committed"); assert.equal((await f.json("read.credential")).expiresAt, 2060);
}));

test("completed recovery and changed token skip POST, including expired refresh recovery", async () => {
  await fixture(async f => {
    await f.put("read.renewal", { base: wire, issued: 2000, completed: true }); await f.put("read.response", reply);
    assert.deepEqual(await f.run({ now: () => 4000 }), { state: "completed" }); assert.equal(f.state.requests, 0); assert.equal(f.state.dispatches, 0);
  });
  await fixture(async f => {
    await f.put("read.credential", { ...wire, token: "rotated" });
    await f.put("read.renewal", "bad orphan");
    const result = await f.run(); assert.equal(result.state, "use_current"); assert.equal(f.state.requests, 0); assert.equal(f.state.dispatches, 0);
  });
});

test("not-due and legacy credentials skip malformed orphan evidence; forced legacy reports auth required", () => fixture(async f => {
  const legacy = { ...wire, refreshToken: null, expiresAt: null, refreshExpiresAt: null, grantId: null };
  await f.put("read.credential", legacy);
  const snapshot = await readCredentialSnapshot(f.root, context, f.resources);
  await f.put("read.renewal", "bad orphan");
  assert.equal((await persistRenewalExchange(f.root, context, snapshot, f.resources)).state, "use_current");
  assert.equal((await persistRenewalExchange(f.root, context, snapshot, f.resources, { force: true })).state, "auth_required");
  assert.equal(f.state.dispatches, 0);
}));

test("fresh preparation replaces incomplete or mismatched evidence and removes its orphan response", async () => {
  for (const journal of [{ base: wire, issued: 1000, completed: false }, { base: { ...wire, token: "other" }, issued: 1000, completed: true }, "malformed journal"]) {
    await fixture(async f => {
      await f.put("read.renewal", journal); await f.put("read.response", { orphan: true });
      assert.equal((await f.run()).state, "completed"); assert.equal((await f.json("read.renewal")).issued, 2000);
    });
  }
});

test("stale login, revocation, cancellation and auth-required gate prevent preparation and POST", async () => {
  for (const [name, value] of [["read.credential", { ...wire, generation: "b".repeat(32) }], ["cancellation", "new-epoch"],
    ["revocation-synthetic.intent", true], ["read.credential", { ...wire, authRequired: true }]] as const) await fixture(async f => {
    await f.put(name, value); const result = await f.run(); assert.ok(result.state === "stopped" || result.state === "auth_required");
    assert.equal(f.state.requests, 0); assert.equal(f.state.dispatches, 0);
  });
});

test("bad completed evidence never starts another POST", async () => {
  for (const invalid of [null, { scope: "upload" }, { ...reply, grantId: "other" }]) await fixture(async f => {
    await f.put("read.renewal", { base: wire, issued: 2000, completed: true }); if (invalid) await f.put("read.response", invalid);
    assert.equal((await f.run()).state, "auth_required"); assert.equal(f.state.requests, 0); assert.equal(f.state.dispatches, 0);
  });
});

test("every POST error is returned without retry and leaves an incomplete journal", async () => {
  for (const category of ["auth_required", "retry_later", "timeout", "request_failed"] as const) await fixture(async f => {
    const result = await f.run({ request: async () => { f.state.requests++; throw new RenewalRequestError(category); } });
    assert.deepEqual(result, { state: "request_failed", reason: category }); assert.equal(f.state.requests, 1);
    assert.equal((await f.json("read.renewal")).completed, false); assert.deepEqual(await f.json("read.credential"), wire);
  });
});

test("new login, cancellation and revocation during POST cannot publish its response", async () => {
  for (const mutate of [async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.credential", { ...wire, generation: "b".repeat(32) }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("cancellation", "new-epoch"),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("revocation-synthetic.intent", true)]) await fixture(async f => {
    let response: Buffer | undefined;
    const result = await f.run({ request: async () => { await mutate(f); return response = Buffer.from(JSON.stringify(reply)); } });
    assert.equal(result.state, "stopped"); assert.equal(f.state.dispatches, 1);
    assert.ok(response?.every(byte => byte === 0)); assert.equal((await f.json("read.renewal")).completed, false);
    await assert.rejects(readFile(f.file("read.response")), { code: "ENOENT" });
  });
});

test("uncertain acknowledgements stop dispatch without cleanup or POST replay", async () => {
  for (const dispatch of [1, 2, 3]) await fixture(async f => {
    f.state.failDispatch = dispatch; const result = await f.run(); assert.equal(result.state, "publication_uncertain");
    assert.equal(f.state.dispatches, dispatch); assert.equal(f.state.requests, dispatch === 1 ? 0 : 1);
    assert.deepEqual(await f.json("read.credential"), wire);
  });
});

test("proven refusals retain honest phase and never mark an unsaved response completed", async () => {
  for (const dispatch of [1, 2, 3]) await fixture(async f => {
    f.state.refusedDispatch = dispatch;
    assert.deepEqual(await f.run(), { state: "stopped", reason: "publication_refused", phase: dispatch === 1 ? "before_request" : dispatch === 2 ? "request_sent" : "response_saved" });
    if (dispatch > 1) assert.equal((await f.json("read.renewal")).completed, false);
  });
});

test("raw malformed response is durable before completion and never silently parsed or retried", () => fixture(async f => {
  let response: Buffer | undefined;
  assert.equal((await f.run({ request: async () => response = Buffer.from("not JSON") })).state, "completed");
  assert.equal(await readFile(f.file("read.response"), "utf8"), "not JSON"); assert.ok(response?.every(b => b === 0));
  assert.equal((await f.run()).state, "auth_required"); assert.equal(f.state.requests, 0);
}));

test("post-completion release failures remain uncertain", () => fixture(async f => {
  f.state.hook = async destination => { if (destination === "read.renewal" && f.state.dispatches === 3) f.state.failRelease = "queue.lock"; };
  assert.equal((await f.run()).state, "publication_uncertain"); assert.equal((await f.json("read.renewal")).completed, true);
}));

test("response mutation during the final credential handoff cannot acquire a completed marker", () => fixture(async f => {
  let responseSaved = false, changed = false;
  f.state.hook = async destination => { if (destination === "read.response") responseSaved = true; };
  f.state.beforeAcl = async handle => {
    if (!responseSaved || changed) return;
    if ((await handle.stat()).ino === (await lstat(f.file("read.credential"))).ino) {
      changed = true; await f.put("read.response", { substituted: true });
    }
  };
  assert.deepEqual(await f.run(), { state: "stopped", phase: "response_saved", reason: "unsafe_file" });
  assert.equal(changed, true); assert.equal((await f.json("read.renewal")).completed, false);
  assert.deepEqual(await f.json("read.response"), { substituted: true }); assert.equal(f.state.dispatches, 2);
}));

test("a new login during preparation prevents the POST after the journal is durable", () => fixture(async f => {
  f.state.hook = async () => f.put("read.credential", { ...wire, generation: "b".repeat(32) });
  assert.deepEqual(await f.run(), { state: "stopped", phase: "before_request", reason: "stale_snapshot" });
  assert.equal(f.state.requests, 0); assert.equal(f.state.dispatches, 1);
}));

test("caller cancellation is fenced before effects and after an owned reply is returned", async () => {
  await fixture(async f => {
    const controller = new AbortController(); controller.abort();
    assert.deepEqual(await f.run({ signal: controller.signal }), { state: "stopped", phase: "before_request", reason: "cancelled" });
    assert.equal(f.state.requests, 0); assert.equal(f.state.dispatches, 0);
  });
  await fixture(async f => {
    const controller = new AbortController(); let response: Buffer | undefined;
    assert.deepEqual(await f.run({ signal: controller.signal, request: async () => {
      controller.abort(); return response = Buffer.from(JSON.stringify(reply));
    } }), { state: "stopped", phase: "request_sent", reason: "cancelled" });
    assert.equal(f.state.dispatches, 1); assert.ok(response?.every(byte => byte === 0));
  });
});

test("coordinator joins the single POST to durable recovery without touching other scopes or config", () => fixture(async f => {
  const result = await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 2000, request: f.request });
  assert.equal(result.state, "credential"); if (result.state !== "credential") return;
  assert.equal(result.source, "renewed"); assert.equal(result.credential.expiresAt, 2060); assert.equal(Object.isFrozen(result.credential), true);
  assert.equal(f.state.requests, 1); assert.equal(f.state.dispatches, 4);
  assert.deepEqual(await f.json("upload.credential"), { retained: "upload" }); assert.deepEqual(await f.json("config"), { retained: true });
  await assert.rejects(lstat(f.file("read.response")), { code: "ENOENT" }); await assert.rejects(lstat(f.file("read.renewal")), { code: "ENOENT" });
}));

test("coordinator marks only the exact terminally rejected record and preserves recovery evidence", async () => {
  for (const request of [async () => { throw new RenewalRequestError("auth_required"); }, async () => Buffer.from("malformed reply"),
    async () => Buffer.from(JSON.stringify({ ...reply, grantId: "wrong-grant" }))]) await fixture(async f => {
    assert.deepEqual(await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 2000, request }), { state: "auth_required" });
    const stored = await f.json("read.credential"); assert.equal(stored.authRequired, true); assert.equal(stored.token, wire.token);
    assert.equal(stored.refreshToken, wire.refreshToken); assert.equal(stored.generation, wire.generation);
    assert.deepEqual(await f.json("upload.credential"), { retained: "upload" }); assert.deepEqual(await f.json("config"), { retained: true });
    assert.equal((await lstat(f.file("read.credential"))).mode & 0o777, 0o600);
    assert.ok((await readdir(f.root)).includes("read.renewal"));
  });
});

test("coordinator reloads a cooperating winner between exchange and recovery without another POST", () => fixture(async f => {
  f.state.afterScopeRelease = async () => assert.equal((await commitCompletedRenewal(f.root, context, f.snapshot, f.resources)).state, "committed");
  const result = await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 2000, request: f.request });
  assert.equal(result.state, "credential"); if (result.state === "credential") { assert.equal(result.source, "other_writer"); assert.equal(result.credential.token, reply.accessToken); }
  assert.equal(f.state.requests, 1); assert.equal(f.state.dispatches, 4);
}));

test("a rejected old token cannot mark a changed credential or a new login", async () => {
  for (const newLogin of [false, true]) await fixture(async f => {
    const next = { ...wire, token: "another-writer-token", ...(newLogin ? { generation: "b".repeat(32) } : {}) };
    const result = await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 2000, request: async () => {
      await f.put("read.credential", next); throw new RenewalRequestError("auth_required");
    } });
    assert.equal(result.state, newLogin ? "stopped" : "credential");
    if (result.state === "credential") { assert.equal(result.source, "other_writer"); assert.equal(result.credential.token, next.token); }
    assert.deepEqual(await f.json("read.credential"), next); assert.equal(f.state.dispatches, 1);
  });
});

test("coordinator never retries nonterminal HTTP or uncertain credential publication", async () => {
  await fixture(async f => {
    let requests = 0;
    assert.deepEqual(await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 2000, request: async () => {
      requests++; throw new RenewalRequestError("retry_later");
    } }), { state: "request_failed", reason: "retry_later" });
    assert.equal(requests, 1); assert.deepEqual(await f.json("read.credential"), wire);
  });
  await fixture(async f => {
    f.state.failDispatch = 4;
    assert.deepEqual(await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 2000, request: f.request }), { state: "publication_uncertain" });
    assert.equal(f.state.requests, 1); assert.equal(f.state.dispatches, 4);
    assert.ok((await readdir(f.root)).includes("read.response")); assert.ok((await readdir(f.root)).includes("read.renewal"));
  });
});

test("cancellation between phases retains completed rotation for a later recovery without POST", () => fixture(async f => {
  const controller = new AbortController(); f.state.afterScopeRelease = async () => controller.abort();
  assert.deepEqual(await renewStateCredential(f.root, context, f.snapshot, f.resources, { signal: controller.signal, now: () => 2000, request: f.request }),
    { state: "stopped", reason: "cancelled" });
  assert.equal((await f.json("read.renewal")).completed, true); assert.deepEqual(await f.json("read.credential"), wire);
  const recovered = await renewStateCredential(f.root, context, f.snapshot, f.resources, { now: () => 4000, request: f.request });
  assert.equal(recovered.state, "credential"); if (recovered.state === "credential") assert.equal(recovered.credential.expiresAt, 2060);
  assert.equal(f.state.requests, 1);
}));

test("terminal marking fences cancellation, intents, input mutation and uncertain acknowledgment", async () => {
  for (const mutation of [async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("cancellation", "changed"),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("revocation-synthetic.intent", true)]) await fixture(async f => {
    await mutation(f); assert.equal((await markCredentialAuthRequired(f.root, context, f.snapshot, f.resources)).state, "unchanged");
    assert.equal(f.state.dispatches, 0); assert.deepEqual(await f.json("read.credential"), wire);
  });
  await fixture(async f => {
    f.state.failDispatch = 1;
    assert.deepEqual(await markCredentialAuthRequired(f.root, context, f.snapshot, f.resources), { state: "publication_uncertain" });
    assert.equal((await f.json("read.credential")).authRequired, true); assert.equal(f.state.dispatches, 1);
  });
  await fixture(async f => {
    const input = { ...f.snapshot, credential: { ...f.snapshot.credential } };
    assert.deepEqual(await renewStateCredential(f.root, context, input, f.resources, { now: () => 2000, request: async credential => {
      assert.equal(credential.token, wire.token); input.credential.token = "changed-caller-value";
      input.credential.generation = "b".repeat(32); throw new RenewalRequestError("auth_required");
    } }), { state: "auth_required" });
    assert.equal((await f.json("read.credential")).token, wire.token); assert.equal((await f.json("read.credential")).generation, wire.generation);
  });
});

test("the operation deadline remains active while terminal rejection waits for queue ownership", { timeout: 4500 }, () => fixture(async f => {
  let locks = 0;
  const waiting: NamespaceResources & StateReplacementResources = { ...f.resources, async lock(handle, options) {
    if (++locks !== 3) return f.resources.lock(handle, options);
    try {
      await new Promise<void>(resolve => {
        const done = () => { clearTimeout(fallback); options.signal?.removeEventListener("abort", done); resolve(); };
        const fallback = setTimeout(done, 2500);
        if (options.signal?.aborted) done(); else options.signal?.addEventListener("abort", done, { once: true });
      });
      return options.signal?.aborted ? { state: "cancelled" } : { state: "failed" };
    } finally { await handle.close(); }
  } };
  assert.deepEqual(await renewStateCredential(f.root, context, f.snapshot, waiting, { timeoutMs: 1000, now: () => 2000,
    request: async () => { throw new RenewalRequestError("auth_required"); } }), { state: "stopped", reason: "cancelled" });
  assert.equal(locks, 3); assert.deepEqual(await f.json("read.credential"), wire);
}));

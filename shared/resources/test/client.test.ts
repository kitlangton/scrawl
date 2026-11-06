import assert from "node:assert/strict";
import { test } from "node:test";
import { NativeResources, NativeResourceError } from "../src/index.ts";
import { fixture } from "./helpers.ts";
import { readFile, watch } from "node:fs/promises";

test("lock ownership transfer, held state, release and idempotence", async () => {
  const f = await fixture(); try {
    const handle = await f.handle(), client = new NativeResources(f.provider);
    const result = await client.lock(handle, { timeoutMs: 0 });
    assert.equal(handle.fd, -1, "parent descriptor is consumed before a lease is returned");
    assert.equal(result.state, "acquired"); if (result.state !== "acquired") return;
    result.lease.assertHeld(); assert.equal(result.lease.signal.aborted, false);
    await result.lease.release(); await result.lease.release();
    assert.equal(result.lease.state, "released"); assert.equal(result.lease.signal.aborted, true);
    assert.throws(() => result.lease.assertHeld(), { category: "lease_lost" });
  } finally { await f.close(); }
});
test("busy/pre-cancel/invalid request consume descriptors without inventing leases", async () => {
  const f = await fixture("busy"); try {
    const client = new NativeResources(f.provider), handle = await f.handle();
    assert.deepEqual(await client.lock(handle, { timeoutMs: 10 }), { state: "busy" }); assert.equal(handle.fd, -1);
    const cancelled = new AbortController(); cancelled.abort(); const h = await f.handle();
    assert.deepEqual(await client.lock(h, { timeoutMs: 1, signal: cancelled.signal }), { state: "cancelled" }); assert.equal(h.fd, -1);
    const bad = await f.handle(); await assert.rejects(client.lock(bad, { timeoutMs: -1 }), { category: "invalid_request" }); assert.equal(bad.fd, -1);
  } finally { await f.close(); }
});
test("unexpected helper loss invalidates the lease", async () => {
  const f = await fixture("lost"); try {
    const result = await new NativeResources(f.provider).lock(await f.handle(), { timeoutMs: 0 });
    assert.equal(result.state, "acquired"); if (result.state !== "acquired") return;
    await new Promise<void>(resolve => result.lease.signal.addEventListener("abort", () => resolve(), { once: true }));
    assert.equal(result.lease.state, "lost"); await assert.rejects(result.lease.release(), { category: "lease_lost" });
  } finally { await f.close(); }
});
test("caller cancellation releases a held lease", async () => {
  const f = await fixture(); try {
    const c = new AbortController(), result = await new NativeResources(f.provider).lock(await f.handle(), { timeoutMs: 0, signal: c.signal });
    assert.equal(result.state, "acquired"); if (result.state !== "acquired") return;
    c.abort(); await result.lease.release(); assert.equal(result.lease.state, "released");
  } finally { await f.close(); }
});
test("provider mismatch and malformed protocol expose only safe categories", async () => {
  const f = await fixture("malformed"); try {
    const h = await f.handle(); await assert.rejects(new NativeResources({ ...f.provider, sha256: "0".repeat(64) }).lock(h, { timeoutMs: 0 }), { category: "provider_mismatch" }); assert.equal(h.fd, -1);
    await assert.rejects(new NativeResources(f.provider).lock(await f.handle(), { timeoutMs: 0 }), (e: unknown) => e instanceof NativeResourceError && e.message === e.category && e.category === "protocol_error");
  } finally { await f.close(); }
});
test("publication preserves terminal states; unknown completion is uncertain", async () => {
  for (const scenario of ["published", "delayed-published", "destination_exists", "refused", "publication_uncertain", "silent", "malformed"]) {
    const f = await fixture(scenario); try {
      const h = await f.handle(); const result = await new NativeResources(f.provider).publish(h, "stage", "final", scenario === "silent" ? { timeoutMs: 100 } : {});
      assert.equal(h.fd, -1); assert.deepEqual(result, { state: scenario === "silent" || scenario === "malformed" ? "publication_uncertain" : scenario === "delayed-published" ? "published" : scenario });
    } finally { await f.close(); }
  }
});
test("invalid names are refused before starting a provider", async () => {
  const f = await fixture(); try {
    const client = new NativeResources({ ...f.provider, executable: "/does-not-exist" });
    for (const name of ["../escape", ".", "..", "with/slash", "x".repeat(4097), "bad\0", "\ud800"]) {
      const h = await f.handle(); assert.deepEqual(await client.publish(h, name, "final"), { state: "refused" }); assert.equal(h.fd, -1);
    }
  } finally { await f.close(); }
});

test("publication cancellation distinguishes not-dispatched from unknown completion", async () => {
  const f = await fixture("silent"); try {
    const client = new NativeResources(f.provider), early = new AbortController(); early.abort();
    const h = await f.handle(); assert.deepEqual(await client.publish(h, "stage", "final", { signal: early.signal }), { state: "refused" }); assert.equal(h.fd, -1);
    const late = new AbortController();
    const result = client.publish(await f.handle(), "stage", "final", { signal: late.signal });
    setTimeout(() => late.abort(), 50);
    assert.deepEqual(await result, { state: "publication_uncertain" });
  } finally { await f.close(); }
});

test("replaceFile consumes the parent, validates identities, and fails closed on older or unknown peers", async () => {
  for (const scenario of ["replaced", "destination_changed", "refused", "publication_uncertain", "silent", "malformed"]) {
    const f = await fixture(scenario); try {
      const h = await f.handle();
      // Allow Node startup on CI; only the silent peer tests deadline expiry.
      const result = await new NativeResources(f.provider).replaceFile(h, "stage", "final", { source: { device: "1", inode: "2" }, destination: null }, { timeoutMs: scenario === "silent" ? 100 : 3000 });
      assert.equal(h.fd, -1);
      assert.deepEqual(result, { state: scenario === "silent" || scenario === "malformed" ? "publication_uncertain" : scenario });
    } finally { await f.close(); }
  }
  const f = await fixture(); try {
    const h = await f.handle();
    assert.deepEqual(await new NativeResources(f.provider).replaceFile(h, "stage", "final", { source: { device: "01", inode: "2" }, destination: null }), { state: "refused" });
    assert.equal(h.fd, -1);
  } finally { await f.close(); }
});

test("malformed replacement identities are refused without leaking the parent descriptor", async () => {
  const bad = [Symbol("private"), 1, null, "1".repeat(1000), { toString() { throw new Error("private"); } }];
  const f = await fixture("replaced");
  try {
    for (const device of bad) {
      const handle = await f.handle();
      try {
        const expected = { source: { device, inode: "2" }, destination: null } as unknown as Parameters<NativeResources["replaceFile"]>[3];
        const outcome = await new NativeResources(f.provider).replaceFile(handle, "stage", "final", expected).catch(() => ({ state: "threw" }));
        assert.equal(handle.fd, -1, "invalid identity must consume its descriptor");
        assert.deepEqual(outcome, { state: "refused" });
      } finally { await handle.close().catch(() => {}); }
    }
    const handle = await f.handle();
    try {
      const expected = { get source(): never { throw new Error("private"); }, destination: null };
      assert.deepEqual(await new NativeResources(f.provider).replaceFile(handle, "stage", "final", expected), { state: "refused" });
      assert.equal(handle.fd, -1);
    } finally { await handle.close().catch(() => {}); }
  } finally { await f.close(); }
});

test("replaceFile cancellation is refused before dispatch and uncertain after one dispatch", async () => {
  const earlyFixture = await fixture(); try {
    const controller = new AbortController(); controller.abort(); const handle = await earlyFixture.handle();
    assert.deepEqual(await new NativeResources({ ...earlyFixture.provider, executable: "/does-not-exist" }).replaceFile(handle, "stage", "final", { source: { device: "1", inode: "2" }, destination: null }, { signal: controller.signal }), { state: "refused" });
    assert.equal(handle.fd, -1);
  } finally { await earlyFixture.close(); }
  const lateFixture = await fixture("silent-count"); try {
    const controller = new AbortController(), handle = await lateFixture.handle();
    const events = watch(lateFixture.directory, { signal: AbortSignal.timeout(2000) });
    const ready = (async () => {
      try {
        for await (const event of events) if (event.filename === "peer-count" && await readFile(lateFixture.marker, "utf8").catch(() => "") === "x") {
          controller.abort(); return;
        }
        throw new Error("peer readiness missing");
      } finally { await events.return(); }
    })();
    const result = new NativeResources(lateFixture.provider).replaceFile(handle, "stage", "final", { source: { device: "1", inode: "2" }, destination: null }, { timeoutMs: 2000, signal: controller.signal });
    await ready;
    assert.deepEqual(await result, { state: "publication_uncertain" }); assert.equal(handle.fd, -1);
    assert.equal(await (await import("node:fs/promises")).readFile(lateFixture.marker, "utf8"), "x");
  } finally { await lateFixture.close(); }
  const countFixture = await fixture("replace-count"); try {
    assert.deepEqual(await new NativeResources(countFixture.provider).replaceFile(await countFixture.handle(), "stage", "final", { source: { device: "1", inode: "2" }, destination: null }), { state: "replaced" });
    assert.equal(await (await import("node:fs/promises")).readFile(countFixture.marker, "utf8"), "x");
  } finally { await countFixture.close(); }
});

test("ACL guard consumes its descriptor and returns only checked facts", async () => {
  for (const [scenario, state] of [["published", "accepted"], ["refused", "refused"]] as const) {
    const f = await fixture(scenario); try {
      const h = await f.handle(); assert.deepEqual(await new NativeResources(f.provider).checkAcl(h), { state }); assert.equal(h.fd, -1);
    } finally { await f.close(); }
  }
});
test("single-component Unicode and space-containing names pass to the provider", async () => {
  const f = await fixture(); try {
    assert.deepEqual(await new NativeResources(f.provider).publish(await f.handle(), "résumé stage", "Sam's review 👋.apparent"), { state: "published" });
  } finally { await f.close(); }
});

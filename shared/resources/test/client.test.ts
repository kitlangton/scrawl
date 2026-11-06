import assert from "node:assert/strict";
import { test } from "node:test";
import { NativeResources, NativeResourceError } from "../src/index.ts";
import { fixture } from "./helpers.ts";

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

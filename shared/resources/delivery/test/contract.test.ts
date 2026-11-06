import assert from "node:assert/strict";
import { test } from "node:test";
import { resourceManifest } from "../../src/artifact.ts";
test("each resource producer target seals exact protocol and bytes without runtime compilers", () => {
  for (const target of ["linux-x64", "macos-x64", "macos-arm64"] as const) {
    const manifest = resourceManifest("a".repeat(40), target, new Uint8Array([1]));
    assert.equal(manifest.target, target); assert.equal(manifest.protocol, 1);
    assert.deepEqual(manifest.runtime, { cargo: false, pathLookup: false });
  }
  assert.throws(() => resourceManifest("short", "linux-x64", new Uint8Array([1])));
});

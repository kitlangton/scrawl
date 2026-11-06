import assert from "node:assert/strict";
import { test } from "node:test";
import { writeFile, chmod, symlink, rm } from "node:fs/promises";
import { join } from "node:path";
import { resourceManifest, validateResourceArtifact } from "../src/index.ts";
import { fixture, target } from "./helpers.ts";

test("artifact inventory, exact target, checksum, mode and canonical manifest", async () => {
  const f = await fixture(); try {
    // The protocol fixture is not a native producer; test only its content seal.
    const bytes = await (await import("node:fs/promises")).readFile(f.executable);
    const manifest = resourceManifest("a".repeat(40), target(), bytes), path = join(f.directory, "manifest.json");
    await writeFile(path, JSON.stringify(manifest, null, 2) + "\n", { mode: 0o644 });
    assert.deepEqual(validateResourceArtifact(f.directory, target()), manifest);
    await writeFile(path, JSON.stringify(manifest).replace('"schema":1', '"schema":1,"schema":1'));
    assert.throws(() => validateResourceArtifact(f.directory, target()));
    await writeFile(path, JSON.stringify(manifest, null, 2) + "\n");
    await chmod(f.directory, 0o775); assert.throws(() => validateResourceArtifact(f.directory, target())); await chmod(f.directory, 0o700);
    await chmod(f.executable, 0o777); assert.throws(() => validateResourceArtifact(f.directory, target())); await chmod(f.executable, 0o755);
    await chmod(f.executable, 0o4755); assert.throws(() => validateResourceArtifact(f.directory, target())); await chmod(f.executable, 0o755);
    await writeFile(join(f.directory, "extra"), "unexpected"); assert.throws(() => validateResourceArtifact(f.directory, target())); await rm(join(f.directory, "extra"));
    await rm(f.executable); await symlink(path, f.executable); assert.throws(() => validateResourceArtifact(f.directory, target()));
  } finally { await f.close(); }
});

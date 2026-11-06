import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";

export type ResourceTarget = "linux-x64" | "macos-x64" | "macos-arm64";
export type ResourceArtifact = {
  schema: 1; protocol: 1; sourceSha: string; target: ResourceTarget;
  executable: "apparent-resource-host"; sha256: string; mode: 493;
  runtimeFiles: []; runtime: { cargo: false; pathLookup: false };
  appleSigning: "unsigned-unnotarized";
};
export function resourceManifest(sourceSha: string, target: ResourceTarget, bytes: Uint8Array): ResourceArtifact {
  if (!/^[a-f0-9]{40}$/.test(sourceSha) || !["linux-x64", "macos-x64", "macos-arm64"].includes(target) || !bytes.length) throw new Error("invalid_resource_artifact");
  return { schema: 1, protocol: 1, sourceSha, target, executable: "apparent-resource-host",
    sha256: createHash("sha256").update(bytes).digest("hex"), mode: 0o755,
    runtimeFiles: [], runtime: { cargo: false, pathLookup: false }, appleSigning: "unsigned-unnotarized" };
}
/** Content/target verification only. Assembly must additionally verify the trusted
 * producer, source closure and provenance before admitting an artifact. */
export function validateResourceArtifact(directory: string, target: ResourceTarget): ResourceArtifact {
  const directoryStat = lstatSync(directory);
  if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink() || directoryStat.mode & 0o022) throw new Error("invalid_resource_artifact");
  const entries = readdirSync(directory).sort();
  if (JSON.stringify(entries) !== JSON.stringify(["apparent-resource-host", "manifest.json"])) throw new Error("invalid_resource_artifact");
  for (const name of entries) {
    const stat = lstatSync(resolve(directory, name));
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || stat.mode & 0o022) throw new Error("invalid_resource_artifact");
  }
  const binary = resolve(directory, "apparent-resource-host"), stat = lstatSync(binary);
  if ((stat.mode & 0o7777) !== 0o755 || stat.size > 16 * 1024 * 1024) throw new Error("invalid_resource_artifact");
  const manifestStat = lstatSync(resolve(directory, "manifest.json"));
  if (manifestStat.size > 4096) throw new Error("invalid_resource_artifact");
  const raw = readFileSync(resolve(directory, "manifest.json"), "utf8");
  let actual: ResourceArtifact;
  try { actual = JSON.parse(raw); }
  catch { throw new Error("invalid_resource_artifact"); }
  const expected = resourceManifest(actual.sourceSha, target, readFileSync(binary));
  if (raw !== JSON.stringify(expected, null, 2) + "\n") throw new Error("invalid_resource_artifact");
  return expected;
}

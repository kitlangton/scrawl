import { execFileSync } from "node:child_process";
import { lstatSync, existsSync, mkdirSync, readdirSync, rmSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { resourceManifest, validateResourceArtifact, type ResourceTarget } from "../../src/artifact.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: root, encoding: "utf8" }).trim();
const target = process.env.OWNER_TARGET as ResourceTarget;
const targets: Record<ResourceTarget, { os: string; arch: string; triple: string }> = {
  "linux-x64": { os: "linux", arch: "x64", triple: "x86_64-unknown-linux-gnu" },
  "macos-x64": { os: "darwin", arch: "x64", triple: "x86_64-apple-darwin" },
  "macos-arm64": { os: "darwin", arch: "arm64", triple: "aarch64-apple-darwin" },
};
const info = Object.hasOwn(targets, target) ? targets[target] : undefined;
if (!info || info.os !== process.platform || info.arch !== process.arch) throw new Error("Unsupported native resource producer target");
function frozen(): string {
  if (execFileSync("git", ["status", "--porcelain", "--untracked-files=normal"], { cwd: repository, encoding: "utf8" })) throw new Error("Resource producer requires a clean committed checkout");
  return execFileSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).trim();
}
const source = frozen(), output = resolve(root, "dist"), cargoTarget = resolve(root, ".build/cargo");
// The installed consumer imports the shipped JavaScript, never a warm local dist.
execFileSync("npm", ["--prefix", resolve(root, ".."), "run", "build"], { cwd: repository, stdio: "inherit" });
if (existsSync(output)) {
  const stat = lstatSync(output);
  if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error("Unsafe resource output directory");
  if (readdirSync(output).length) rmSync(output, { recursive: true });
}
mkdirSync(output, { recursive: true, mode: 0o755 });
chmodSync(output, 0o755);
const jobs = process.env.T3_BUILD_JOBS ?? "3";
if (!/^[1-9][0-9]*$/.test(jobs)) throw new Error("Invalid admitted Cargo job count");
const cargo = ["--manifest-path", resolve(repository, "shared/resources/native/Cargo.toml"), "--locked", "--target", info.triple, "--target-dir", cargoTarget, "-j", jobs];
if (process.env.APPARENT_CARGO_OFFLINE === "1") cargo.push("--offline");
execFileSync("cargo", ["test", ...cargo], { cwd: repository, stdio: "inherit" });
execFileSync("cargo", ["build", ...cargo, "--release"], { cwd: repository, stdio: "inherit" });
const bytes = readFileSync(resolve(cargoTarget, info.triple, "release/apparent-resource-host"));
const executable = resolve(output, "apparent-resource-host");
writeFileSync(executable, bytes, { flag: "wx", mode: 0o755 }); chmodSync(executable, 0o755);
writeFileSync(resolve(output, "manifest.json"), JSON.stringify(resourceManifest(source, target, bytes), null, 2) + "\n", { flag: "wx", mode: 0o644 });
validateResourceArtifact(output, target);
// Exercise the public TypeScript adapter against these exact staged bytes.
const nativeTests = ["shared/resources/test/native.test.ts"];
// ARM is an owner-local producer option, not a registered installed-provider target.
if (target !== "macos-arm64") nativeTests.push("shared/resources/test/installed-native.test.ts");
execFileSync(process.execPath, ["--experimental-strip-types", "--test", ...nativeTests], {
  cwd: repository, stdio: "inherit", env: { ...process.env, APPARENT_RESOURCE_HOST_BIN: executable },
});
if (frozen() !== source) throw new Error("Source changed during resource producer verification");
validateResourceArtifact(output, target);
console.log(`Verified resource helper: ${target}, source ${source}`);

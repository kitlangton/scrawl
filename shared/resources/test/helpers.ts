import { createHash } from "node:crypto";
import { chmod, mkdtemp, open, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { ResourceProvider, ResourceTarget } from "../src/index.ts";
const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
export const target = (): ResourceTarget => process.platform === "linux" ? "linux-x64" : process.arch === "arm64" ? "macos-arm64" : "macos-x64";
export async function provider(executable: string): Promise<ResourceProvider> {
  return { executable, protocol: 1, target: target(), sha256: createHash("sha256").update(await readFile(executable)).digest("hex") };
}
export async function fixture(scenario = "published") {
  const directory = await mkdtemp(join(tmpdir(), "apparent-resources-")); await chmod(directory, 0o700);
  const executable = join(directory, "apparent-resource-host");
  await writeFile(executable, `#!/bin/sh\nexec ${quote(process.execPath)} --experimental-strip-types ${quote(fileURLToPath(new URL("./peer.ts", import.meta.url)))} ${quote(scenario)} "$@"\n`, { mode: 0o755 });
  return { directory, executable, provider: await provider(executable),
    handle: async () => open(join(directory, "lock"), "a+", 0o600),
    close: () => rm(directory, { recursive: true, force: true }) };
}

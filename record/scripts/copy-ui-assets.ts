import { copyFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const packageRoot = new URL("../", import.meta.url);
const output = new URL("dist/ui/", packageRoot);
await mkdir(fileURLToPath(output), { recursive: true });
await copyFile(
  fileURLToPath(new URL("../shared/assets/fonts/HostGrotesk-Variable.ttf", packageRoot)),
  fileURLToPath(new URL("HostGrotesk-Variable.ttf", output)),
);

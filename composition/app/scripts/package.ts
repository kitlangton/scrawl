import assert from 'node:assert/strict';
import { packager } from '@electron/packager';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyAssembly } from './assemble.ts';

/** Host-only development bundle. Uses published Electron, never a native source build. */
export async function packageApplication(assembly: string, output: string) {
  assert(process.platform === 'linux' || process.platform === 'darwin', 'Supported hosts are Linux and macOS');
  assert(process.arch === 'x64' || process.arch === 'arm64', 'Supported architectures are x64 and arm64');
  await verifyAssembly(assembly);
  const packages = await packager({ dir: resolve(assembly), out: resolve(output),
    name: 'apparent-foundation', executableName: 'apparent-foundation',
    appBundleId: 'com.tryapparent.foundation', electronVersion: '45.2.1',
    platform: process.platform, arch: process.arch, overwrite: false, prune: false,
    asar: false, quiet: true });
  assert.equal(packages.length, 1, 'Expected one fresh application bundle');
  // Packager must retain the verified payload byte-for-byte.
  const resources = process.platform === 'darwin'
    ? resolve(packages[0], 'Contents/Resources/app') : resolve(packages[0], 'resources/app');
  await verifyAssembly(resources);
  return packages[0];
}
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const [assembly, output] = process.argv.slice(2);
  assert(assembly && output, 'Usage: package.ts <assembly> <new-package-directory>');
  console.log(await packageApplication(assembly, output));
}

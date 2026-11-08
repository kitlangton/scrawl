import assert from 'node:assert/strict';
import { packager } from '@electron/packager';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyAssembly } from './assemble.ts';
import { ELECTRON_VERSION, pinnedElectronZip } from './runtime/electron.ts';

/** Host-only development bundle. Uses published Electron, never a native source build. */
export async function packageApplication(assembly: string, output: string) {
  assert(process.platform === 'linux' || process.platform === 'darwin', 'Supported hosts are Linux and macOS');
  assert(process.arch === 'x64' || process.arch === 'arm64', 'Supported architectures are x64 and arm64');
  await verifyAssembly(assembly);
  const zip = await pinnedElectronZip({ platform: process.platform, arch: process.arch });
  const packages = await packager({ dir: resolve(assembly), out: resolve(output),
    name: 'apparent-foundation', executableName: 'apparent-foundation',
    appBundleId: 'com.tryapparent.foundation', electronVersion: ELECTRON_VERSION, electronZipDir: dirname(zip),
    platform: process.platform, arch: process.arch, overwrite: false, prune: false,
    asar: false, quiet: true });
  assert.equal(packages.length, 1, 'Expected one fresh application bundle');
  return verifyPackagedApplication(packages[0], process.platform);
}

/** Packager returns a distribution directory; macOS nests its .app within it. */
export async function verifyPackagedApplication(directory: string, platform: 'linux' | 'darwin') {
  const application = platform === 'darwin' ? resolve(directory, 'apparent-foundation.app') : resolve(directory);
  const resources = platform === 'darwin'
    ? resolve(application, 'Contents/Resources/app') : resolve(application, 'resources/app');
  await verifyAssembly(resources);
  return application;
}
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const [assembly, output] = process.argv.slice(2);
  assert(assembly && output, 'Usage: package.ts <assembly> <new-package-directory>');
  console.log(await packageApplication(assembly, output));
}

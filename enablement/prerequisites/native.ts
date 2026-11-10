// Fail before Cargo; this is input/executable readiness, never native acceptance.
import { accessSync, constants, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function checkNativeInputs(root: string, platform = process.platform): string {
  const frontend = resolve(root, 'capture/desktop/ui/frontend');
  const require = createRequire(resolve(frontend, 'package.json'));
  const electron: unknown = require('electron');
  if (typeof electron !== 'string' || !electron.startsWith(frontend + '/node_modules/electron/')) {
    throw new Error('Electron did not resolve to the locked local installation');
  }
  accessSync(electron, constants.X_OK);
  if (!statSync(electron).isFile()) throw new Error('Electron executable is not a file');
  const inputs = [
    'capture/desktop/ui/frontend/tests/reference-native.cjs',
    'capture/desktop/ui/frontend/electron/main.cjs',
    'capture/desktop/ui/frontend/electron/preload.cjs',
    'capture/desktop/ui/frontend/electron/runtime.cjs',
    'capture/desktop/ui/frontend/electron/connect.cjs',
    'capture/desktop/ui/frontend/electron/storage-policy.cjs',
    'capture/desktop/ui/frontend/electron/record-completion/worker.js',
    'capture/desktop/ui/frontend/electron/record-completion/policy.js',
    'capture/desktop/ui/frontend/electron/record-completion/package.json',
  ];
  if (platform === 'darwin') inputs.push(
    'capture/desktop/ui/frontend/tests/mac-storage-native.cjs',
    'capture/desktop/scripts/native/copy-capture-id.swift',
    'capture/desktop/assets/Apparent.icns',
    'shared/assets/fonts/HostGrotesk-Variable.ttf',
    'shared/assets/brand/apparent-wordmark.png',
    'shared/assets/brand/apparent-a.png',
    'shared/assets/brand/connect-frogs.png',
  );
  for (const input of inputs) {
    const file = resolve(root, input);
    accessSync(file, constants.R_OK);
    if (!statSync(file).isFile() || statSync(file).size === 0) {
      throw new Error('Required native input is empty or not a file: ' + input);
    }
  }
  return electron;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  // The existing workflow invokes this entrypoint after the locked frontend
  // install and before Cargo. Prepare the TS-owned artifact here as well as in
  // owner-local build/test commands; a clean checkout has no committed CJS donor.
  await import('../../capture/desktop/ui/frontend/scripts/build-connect-transport.ts');
  // Rust-only provider jobs need Record's packaged worker even without a
  // renderer build. Use the same generator as Desktop build and staging.
  await import('../../capture/desktop/ui/frontend/scripts/build-pill.ts');
  console.log('Native prerequisites ready: ' + checkNativeInputs(process.cwd()));
}

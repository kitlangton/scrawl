import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const frontend = resolve(fileURLToPath(new URL('..', import.meta.url)));
const repository = resolve(frontend, '../../../..');

export const SHARED_UI_FRESHNESS_FILES = ['button.tsx', 'tabs.tsx', 'utils.ts', 'icons.ts', 'theme.css', 'tokens.css', 'window-theme.cjs', 'status-card.tsx', 'disclosure-menu.tsx', 'status.css', 'feedback/alert.tsx', 'feedback/collapsible.tsx', 'build-identity/index.tsx', 'build-identity/label.ts', 'build-identity/build-identity.css'];
export const RECORD_FRESHNESS_FILES = [
  'package.json', 'dist/ui/index.js', 'dist/ui/index.d.ts',
  'dist/ui/recording-pill.js', 'dist/ui/recording-pill.d.ts',
  'dist/ui/UploadPocket.js', 'dist/ui/UploadPocket.d.ts',
  'dist/ui/types.d.ts', 'dist/ui/style.css', 'dist/ui/HostGrotesk-Variable.ttf',
];

export function checkDependencyFreshness(paths = { frontend, repository }): void {
  for (const file of SHARED_UI_FRESHNESS_FILES) {
    const canonical = readFileSync(resolve(paths.repository, 'shared/ui/src', file));
    const installed = readFileSync(resolve(paths.frontend, 'node_modules/@apparent/ui/src', file));
    if (!canonical.equals(installed)) throw new Error('Shared UI changed. Run npm ci before building.');
  }

  for (const file of RECORD_FRESHNESS_FILES) {
    const canonical = readFileSync(resolve(paths.repository, 'record', file));
    const installed = readFileSync(resolve(paths.frontend, 'node_modules/@apparent/record', file));
    if (!canonical.equals(installed)) throw new Error('Record package changed. Run npm ci in record, then npm ci here before building.');
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  checkDependencyFreshness();
}

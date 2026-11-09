import {appendFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
import {changedPaths as inputPaths} from './inputs.ts';

// Keep the previous automatic Public/updater input sets. Changes to this
// orchestration also exercise both consumers before they land.
export function select(paths: string[] | null): {public: boolean; updater: boolean} {
  if (paths === null) return {public: true, updater: true};
  const orchestration = paths.some(path => path === '.github/workflows/checks.yml' || path.startsWith('enablement/ci/'));
  return {
    public: orchestration || paths.some(path =>
      ['public/site/', 'shared/ui/', 'shared/analytics/', 'shared/assets/'].some(prefix => path.startsWith(prefix)) ||
      path === '.github/workflows/stream-public.yml'),
    updater: orchestration || paths.some(path =>
      ['capture/desktop/scripts/linux/gui/', 'capture/desktop/scripts/linux/updater/'].some(prefix => path.startsWith(prefix)) ||
      ['.github/workflows/linux-updater-harness.yml', 'capture/desktop/linux/update.rs',
        'capture/desktop/settings/settings.rs', 'capture/desktop/settings/settings_window.rs'].includes(path)),
  };
}

export function changedPaths(base: string, head: string): string[] | null {
  if (!/^[a-f0-9]{40}$/.test(base) || /^0+$/.test(base) || !/^[a-f0-9]{40}$/.test(head)) return null;
  try {
    return inputPaths(base, head, process.cwd());
  } catch {
    // Missing history cannot suppress checks.
    return null;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const result = select(changedPaths(process.env.BASE_SHA ?? '', process.env.GITHUB_SHA ?? ''));
  appendFileSync(process.env.GITHUB_OUTPUT!, Object.entries(result).map(([key, value]) => `${key}=${value}\n`).join(''));
}

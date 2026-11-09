import {appendFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
import {changedPaths as inputPaths} from './inputs.ts';

// Only known prose locations are exempt. Agent instructions and embedded or
// test/fixture Markdown remain executable inputs, even with a .md extension.
export function documentationOnlyPath(path: string): boolean {
  if (!path.endsWith('.md')) return false;
  if (/(^|\/)(AGENTS|SKILL|CLAUDE|GEMINI)\.md$/i.test(path) ||
      /(^|\/)(skills?|prompts?|instructions?|tests?|fixtures?|templates?)(\/|$)/i.test(path) ||
      path === 'README.md') return false;
  const owners = ['capture/engine', 'capture/desktop', 'capture/agent', 'connect', 'public/site', 'shared', 'enablement'];
  return path === 'CONTEXT.md' || path === 'VERIFICATION.md' ||
    path === 'enablement/refactor.md' ||
    owners.some(owner => path.startsWith(owner + '/docs/') ||
      ['README', 'CHANGELOG', 'CONTRIBUTING', 'LICENSE'].some(name => path === owner + '/' + name + '.md'));
}

export function needsPlatform(paths: string[] | null): boolean {
  return paths === null || paths.length === 0 || paths.some(path => !documentationOnlyPath(path));
}

// Keep the previous automatic Public/updater input sets. Changes to this
// orchestration also exercise both consumers before they land.
export function select(paths: string[] | null): {public: boolean; updater: boolean} {
  if (paths === null) return {public: true, updater: true};
  paths = paths.filter(path => !documentationOnlyPath(path));
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

// Exact portable Settings renderer inputs, not a directory/extension exemption.
// Shell, bridge, styles, onboarding, notes and unknown inputs retain Mac proof.
const portableSettings = new Set([
  'storage.tsx', 'retention.tsx', 'connect-settings.tsx',
  'error-reporting.tsx', 'reference.tsx',
].map(name => 'capture/desktop/ui/frontend/src/' + name));

export function requiresMac(paths: string[] | null, nonModifications: string[] | null = []): boolean {
  return paths === null || paths.length === 0 || nonModifications === null || nonModifications.length > 0 ||
    paths.some(path => !portableSettings.has(path));
}

export function changedPaths(base: string, head: string, nonModificationsOnly = false): string[] | null {
  if (!/^[a-f0-9]{40}$/.test(base) || /^0+$/.test(base) || !/^[a-f0-9]{40}$/.test(head)) return null;
  try {
    return inputPaths(base, head, process.cwd(), nonModificationsOnly);
  } catch {
    // Missing history cannot suppress checks.
    return null;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const base = process.env.BASE_SHA ?? '';
  const head = process.env.GITHUB_SHA ?? '';
  const paths = changedPaths(base, head);
  const result = {...select(paths), macos: requiresMac(paths, changedPaths(base, head, true)), platform: needsPlatform(paths)};
  appendFileSync(process.env.GITHUB_OUTPUT!, Object.entries(result).map(([key, value]) => `${key}=${value}\n`).join(''));
}

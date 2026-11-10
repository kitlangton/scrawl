import {execFileSync} from 'node:child_process';
import {appendFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
import {isDeepStrictEqual} from 'node:util';
import {changedPaths as inputPaths} from './inputs.ts';

// Public's agent-files.ts reads these bodies, not merely links to them.
// Build inputs cannot use the ordinary-prose exemption.
const publicMarkdown = new Set([
  'README.md', 'capture/agent/skills/apparent/SKILL.md',
  'capture/agent/docs/usage.md', 'capture/desktop/docs/guides/recording.md',
  'capture/engine/docs/guides/processing.md',
]);

// Only known prose locations are exempt. Agent instructions and embedded or
// test/fixture Markdown remain executable inputs, even with a .md extension.
export function documentationOnlyPath(path: string): boolean {
  if (!path.endsWith('.md') || publicMarkdown.has(path)) return false;
  if (/(^|\/)(AGENTS|SKILL|CLAUDE|GEMINI)\.md$/i.test(path) ||
      /(^|\/)(skills?|prompts?|instructions?|tests?|fixtures?|templates?)(\/|$)/i.test(path)) return false;
  const owners = ['capture/engine', 'capture/desktop', 'capture/agent', 'connect', 'public/site', 'shared', 'enablement'];
  return path === 'CONTEXT.md' || path === 'VERIFICATION.md' ||
    path === 'enablement/refactor.md' ||
    owners.some(owner => path.startsWith(owner + '/docs/') ||
      ['README', 'CHANGELOG', 'CONTRIBUTING', 'LICENSE'].some(name => path === owner + '/' + name + '.md'));
}

export function needsPlatform(paths: string[] | null): boolean {
  return paths === null || paths.length === 0 || paths.some(path => !documentationOnlyPath(path));
}

// Preserve main's Public/updater input sets and ordinary-prose exception.
export function select(paths: string[] | null): {public: boolean; updater: boolean} {
  if (paths === null) return {public: true, updater: true};
  paths = paths.filter(path => !documentationOnlyPath(path));
  const orchestration = paths.some(path => path === '.github/workflows/checks.yml' || path.startsWith('enablement/ci/'));
  return {
    public: orchestration || paths.some(path =>
      publicMarkdown.has(path) ||
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

const portal = 'capture/desktop/linux/capture/portal.rs';
// Audited cfg(linux) leaf adapter; no shared policy or Mac compiler consumer.
const linuxAdapters = new Set(['capture/desktop/linux/integration/appearance.rs']);
// Launch-free test drivers, not shipped runtime/package inputs. Keep their
// build/version watches: selection avoids unrelated PR verification, not stamping.
const linuxDrivers = new Set([
  'capture-smoke.py', 'gui-vm.py', 'gui-vm-reaper.py',
  'test-capture-smoke.py', 'test-gui-vm.py',
].map(name => 'capture/desktop/scripts/linux/gui/' + name));
const linuxDoc = 'capture/desktop/docs/linux/linux.md';

export function macDecision(paths: string[] | null, nonModifications: string[] | null = []) {
  if (paths === null || nonModifications === null) return {macos: true, reasons: ['mac:unknown-history']};
  if (!paths.length) return {macos: true, reasons: ['mac:empty-delta']};
  const reasons = paths.map(path => {
    if (nonModifications.includes(path)) return 'mac:unsupported-change:' + path;
    if (linuxAdapters.has(path)) return 'linux:reviewed-adapter:' + path;
    if (linuxDrivers.has(path)) return 'linux:reviewed-test-driver:' + path;
    if (portableSettings.has(path)) return 'linux:reviewed-settings:' + path;
    // Audited cfg(linux) module and guide; no Mac build/package consumer.
    if (path === portal || path === linuxDoc) return 'linux:reviewed-portal-input:' + path;
    return 'mac:unproven-input:' + path;
  });
  for (const path of nonModifications) if (!paths.includes(path)) reasons.push('mac:unsupported-change:' + path);
  return {macos: reasons.some(reason => reason.startsWith('mac:')), reasons};
}

export function requiresMac(paths: string[] | null, nonModifications: string[] | null = []): boolean {
  return macDecision(paths, nonModifications).macos;
}

type Change = {path: string; status: string; oldMode: string; newMode: string};
const git = (args: string[]) => execFileSync('git', args,
  {encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe']});

function changes(base: string, head: string): Change[] | null {
  if (!/^[a-f0-9]{40}$/.test(base) || /^0+$/.test(base) || !/^[a-f0-9]{40}$/.test(head)) return null;
  try {
    if (git(['rev-parse', '--is-shallow-repository']).trim() !== 'false') return null;
    git(['merge-base', '--is-ancestor', base, head]);
    const sharedPaths = inputPaths(base, head, process.cwd());
    // No rename folding: both owner sides remain verification inputs.
    const raw = git(['diff', '--no-renames', '--no-ext-diff', '--no-textconv', '--ignore-submodules=none',
      '--raw', '--no-abbrev', '-z', base, head, '--']);
    if (!raw) return sharedPaths.length ? null : [];
    const parts = raw.split('\0');
    if (parts.pop() !== '' || parts.length % 2 !== 0) return null;
    const result: Change[] = [];
    for (let i = 0; i < parts.length; i += 2) {
      const match = /^:(\d{6}) (\d{6}) [a-f0-9]{40} [a-f0-9]{40} ([A-Z])$/.exec(parts[i]);
      if (!match || !parts[i + 1]) return null;
      const [, oldMode, newMode, status] = match;
      result.push({path: parts[i + 1], status, oldMode, newMode});
    }
    // Shared path facts must agree with the additional raw status/mode proof.
    return isDeepStrictEqual(sharedPaths, result.map(change => change.path)) ? result : null;
  } catch {
    return null;
  }
}

const unsupported = (change: Change) => change.status !== 'M' || change.oldMode !== '100644' || change.newMode !== '100644';

// Main's prose exception permits regular additions/deletions and both regular
// rename sides. A symlink, executable or unknown mode is not proven prose.
const regularProseChange = (change: Change) =>
  (change.status === 'M' && change.oldMode === '100644' && change.newMode === '100644') ||
  (change.status === 'A' && change.oldMode === '000000' && change.newMode === '100644') ||
  (change.status === 'D' && change.oldMode === '100644' && change.newMode === '000000');

export function changedPaths(base: string, head: string, nonModificationsOnly = false): string[] | null {
  return changes(base, head)?.filter(change => !nonModificationsOnly || unsupported(change)).map(change => change.path) ?? null;
}

export function selectRange(base: string, head: string) {
  const delta = changes(base, head);
  const paths = delta?.map(change => change.path) ?? null;
  const unsafeProse = delta?.some(change => documentationOnlyPath(change.path) && !regularProseChange(change)) ?? false;
  const mac = macDecision(paths, delta?.filter(unsupported).map(change => change.path) ?? null);
  const platform = needsPlatform(paths) || unsafeProse;
  return {...select(unsafeProse ? null : paths), ...mac, platform,
    reasons: platform ? mac.reasons : paths!.map(path => 'docs:reviewed-prose:' + path)};
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const {reasons, ...result} = selectRange(process.env.BASE_SHA ?? '', process.env.GITHUB_SHA ?? '');
  console.log(JSON.stringify({...result, reasons}));
  appendFileSync(process.env.GITHUB_OUTPUT!, Object.entries(result).map(([key, value]) => key + '=' + value + '\n').join(''));
}

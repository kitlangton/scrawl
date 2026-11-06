#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const owners = ['agent', 'app-shell', 'record', 'onboarding', 'resources', 'connect-client', 'public-site', 'browser'] as const;
type Owner = typeof owners[number];

function versionTuple(value: string): number[] {
  const match = value.match(/\d+(?:\.\d+){1,2}/);
  if (!match) throw new Error(`Could not parse tool version '${value}'.`);
  return match[0].split('.').map(Number);
}

export function versionAtLeast(actual: string, minimum: string): boolean {
  const a = versionTuple(actual);
  const b = versionTuple(minimum);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
  }
  return true;
}

export function parseArgs(args: string[]): { owners: Owner[]; help: boolean } {
  const selected: Owner[] = [];
  let help = false;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--help' || args[i] === '-h') help = true;
    else if (args[i] === '--owner') {
      const owner = args[++i];
      if (!owners.includes(owner as Owner)) throw new Error(`Unknown owner '${owner ?? ''}'. Choose: ${owners.join(', ')}`);
      if (!selected.includes(owner as Owner)) selected.push(owner as Owner);
    } else throw new Error(`Unknown argument '${args[i]}'. Use --owner <name> or --help.`);
  }
  return { owners: selected, help };
}

const packages: Record<Owner | 'baseline', string[]> = {
  baseline: ['composition/architecture'],
  agent: ['agent'],
  'app-shell': ['shared/contracts', 'composition/app'],
  record: ['record', 'capture/desktop/ui/frontend'],
  onboarding: ['onboarding'],
  resources: ['shared/resources'],
  'connect-client': ['connect/delivery/client'],
  'public-site': ['public/site'],
  browser: ['connect/web'],
};
const importChecks: Partial<Record<Owner | 'baseline', [string, string][]>> = {
  baseline: [['composition/architecture', 'typescript']],
  agent: [['agent', '@apparent/engine-sdk']],
  'app-shell': [['composition/app', '@apparent/app-contracts']],
  record: [['record', '@apparent/record']],
  onboarding: [['onboarding', 'typescript']],
  resources: [['shared/resources', 'typescript']],
  'connect-client': [['connect/delivery/client', 'typescript']],
  'public-site': [['public/site', 'vite']],
  browser: [['connect/web', 'vite']],
};

function run(command: string, args: string[], cwd: string, env: NodeJS.ProcessEnv = process.env): void {
  const result = spawnSync(command, args, { cwd, env, stdio: 'inherit' });
  if (result.error) throw new Error(`${command} is unavailable: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed with ${result.status ?? result.signal}. Check the diagnostic above, then rerun this command after fixing the tool/network/lock issue.`);
}

function toolVersion(command: string, args: string[], cwd: string): string {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8' });
  if (result.error) throw new Error(`${command} is required but unavailable: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed with ${result.status ?? result.signal}: ${result.stderr.trim()}`);
  return result.stdout.trim();
}

function checkToolchain(root: string, owner: Owner | 'baseline'): void {
  if (!versionAtLeast(process.versions.node, '22.18.0')) throw new Error(`Node ${process.versions.node} is too old; this repository setup requires Node >=22.18.0.`);
  const python = toolVersion('python3', ['--version'], root);
  if (!versionAtLeast(python, '3.9.0')) throw new Error(`${python} is too old; dependency preflight requires Python >=3.9.0.`);
  const usesBun = packages[owner].some((relative) => relative === 'public/site' || relative === 'connect/web');
  if (usesBun) {
    const actual = toolVersion('bun', ['--version'], root);
    if (actual !== '1.3.10') throw new Error(`Bun ${actual} found; selected locked browser packages require Bun 1.3.10.`);
  } else {
    const npm = toolVersion('npm', ['--version'], root);
    if (!versionAtLeast(npm, '9.0.0')) throw new Error(`npm ${npm} is too old for the committed lockfiles; require npm >=9.0.0.`);
  }
}

function preflight(root: string, owner: Owner | 'baseline'): boolean {
  const py = String.raw`
import importlib.util, pathlib, sys
root, owner = pathlib.Path(sys.argv[1]), sys.argv[2]
spec = importlib.util.spec_from_file_location('apparent_verification', root/'enablement/verification.py')
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
paths = {'baseline':['composition/app/src/main.tsx'], 'agent':['agent/package.json'], 'app-shell':['composition/app/package.json'], 'record':['record/package.json'], 'onboarding':['onboarding/package.json'], 'resources':['shared/resources/package.json'], 'connect-client':['connect/delivery/client/src/index.ts'], 'public-site':['public/site/package.json'], 'browser':['connect/web/package.json']}[owner]
selection = {key: False for key in ('agent','app_shell','record','onboarding_package','platform_resources','public_site','browser')}
selection.update(paths=paths)
keys = {'agent':'agent','app-shell':'app_shell','record':'record','onboarding':'onboarding_package','resources':'platform_resources','connect-client':'connect_client','public-site':'public_site','browser':'browser'}
if owner in keys: selection[keys[owner]] = True
module.ROOT = root
try:
 module.preflight_dependencies(selection, include_architecture=True, include_browser=(owner == 'browser'))
except Exception as error:
 print(error, file=sys.stderr); sys.exit(1)
`;
  const result = spawnSync('python3', ['-c', py, root, owner], { cwd: root, encoding: 'utf8' });
  if (result.error) throw new Error(`Python 3 is required for the existing read-only dependency preflight: ${result.error.message}`);
  if (result.status !== 0) {
    process.stderr.write(result.stderr || result.stdout);
    return false;
  }
  return true;
}

function install(root: string, owner: Owner | 'baseline'): void {
  for (const relative of packages[owner]) {
    const cwd = resolve(root, relative);
    if (!existsSync(resolve(cwd, 'package.json'))) throw new Error(`Missing ${relative}/package.json; checkout may be incomplete.`);
    if (relative === 'public/site' || relative === 'connect/web') {
      if (!existsSync(resolve(cwd, 'bun.lock'))) throw new Error(`Missing ${relative}/bun.lock; refusing an unlocked install.`);
      run('bun', ['install', '--frozen-lockfile', '--prefer-offline'], cwd);
    } else {
      if (!existsSync(resolve(cwd, 'package-lock.json'))) throw new Error(`Missing ${relative}/package-lock.json; refusing an unlocked install.`);
      run('npm', ['ci', '--prefer-offline', '--no-audit', '--no-fund'], cwd);
    }
  }
}

function verifyImports(root: string, owner: Owner | 'baseline'): void {
  for (const [relative, name] of importChecks[owner] ?? []) {
    run('node', ['--input-type=module', '-e', `await import(${JSON.stringify(name)})`], resolve(root, relative));
  }
}

/** A crashed holder requires explicit recovery; never delete another process's lock. */
export function withPreparationLock(root: string, action: () => void, timeoutMs = 120_000): void {
  const lock = resolve(root, '.apparent-worktree-prepare.lock');
  const deadline = Date.now() + timeoutMs;
  while (true) {
    try {
      mkdirSync(lock, { mode: 0o700 });
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
      if (Date.now() >= deadline) throw new Error(`Preparation is already locked at ${lock}. Wait for the existing setup terminal; after a crash, verify no preparation is running before removing this empty lock directory.`);
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
    }
  }
  try { action(); } finally { rmdirSync(lock); }
}

function prepareUnlocked(root: string, selected: Owner[]): void {
  for (const owner of ['baseline', ...selected] as const) {
    console.log(`\n[prepare] ${owner}: checking locked install and existing freshness guards`);
    checkToolchain(root, owner);
    // Existing Bun preflights check prerequisites, not installed lock freshness.
    // Let Bun validate its frozen graph on every selected browser preparation.
    if (owner !== 'browser' && owner !== 'public-site' && preflight(root, owner)) {
      console.log(`[prepare] ${owner}: already prepared; no installation needed`);
      verifyImports(root, owner);
      continue;
    }
    console.log(`[prepare] ${owner}: installing from the committed lockfile using local download caches where possible`);
    install(root, owner);
    if (!preflight(root, owner)) throw new Error(`[prepare] ${owner}: installed tree did not pass the existing dependency preflight/freshness checks`);
    verifyImports(root, owner);
    console.log(`[prepare] ${owner}: prepared and verified`);
  }
}

export function prepare(root: string, selected: Owner[]): void {
  withPreparationLock(root, () => prepareUnlocked(root, selected));
}

function main(): void {
  try {
    const parsed = parseArgs(process.argv.slice(2));
    if (parsed.help) {
      console.log(`Usage: node --experimental-strip-types enablement/worktree-setup/prepare.ts [--owner ${owners.join('|')}]\nWithout --owner, prepares only the shared architecture-check baseline.`);
      return;
    }
    const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
    prepare(root, parsed.owners);
  } catch (error) {
    console.error(`[prepare] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) main();

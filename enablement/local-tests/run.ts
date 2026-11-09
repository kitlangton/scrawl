// Local owner-level gates. Native/release proof remains separately required.
import { spawnSync } from 'node:child_process';
import { readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {changedPaths as inputPaths, workflowControlPaths} from '../ci/inputs.ts';

export const root = fileURLToPath(new URL('../../', import.meta.url));
export const owners = ['capture', 'engine', 'desktop', 'capture-tools', 'client', 'web', 'service', 'public', 'hermes', 'tooling'] as const;
type Owner = typeof owners[number];
export type Command = { argv: string[]; cwd: string; env?: Record<string, string> };

export function select(paths: string[], known = true, workflowControls: string[] = []): Owner[] {
  const selected = new Set<Owner>();
  const add = (...values: Owner[]) => values.forEach(value => selected.add(value));
  if (!known || !paths.length) add(...owners);
  for (const path of paths) {
    if (['README.md', 'AGENTS.md', 'CONTEXT.md', 'enablement/refactor.md'].includes(path)) continue;
    if (known && workflowControls.includes(path)) add('tooling');
    else if (path.startsWith('enablement/') || path.startsWith('.githooks/')) add('tooling');
    else if (path.startsWith('hermes/')) add('hermes');
    else if (path.startsWith('connect/web/')) add('web', 'public');
    else if (path.startsWith('connect/client/')) add('client', 'capture');
    else if (path.startsWith('connect/service/')) add('service', 'client', 'web', 'public', 'capture');
    else if (path.startsWith('public/site/public/assets/')) add('public', 'web');
    else if (path.startsWith('public/site/')) add('public');
    else if (path.startsWith('capture/desktop/scripts/') || path.startsWith('capture/agent/scripts/')) add('capture', 'engine', 'capture-tools');
    else if (path.startsWith('capture/desktop/ui/frontend/')) add('desktop', 'capture');
    else if (path.startsWith('capture/desktop/')) add('desktop', 'capture', 'engine', 'capture-tools');
    else if (path.startsWith('capture/')) add('capture', 'engine');
    else if (path.startsWith('composition/')) add('capture', 'engine');
    else if (path.startsWith('shared/ui/')) add('desktop', 'service', 'capture');
    else add(...owners); // Shared contracts, root build/workflows and unknown owners.
  }
  return owners.filter(owner => selected.has(owner));
}

export function changedPaths(base: string, head: string, cwd = root): string[] {
  return inputPaths(base, head, cwd);
}

export function selection(base: string, head: string, cwd = root, conservativeWorkflows = false) {
  let paths: string[] = [], historyKnown = true;
  try { paths = changedPaths(base, head, cwd); } catch { historyKnown = false; }
  const workflowControls = historyKnown && !conservativeWorkflows ? workflowControlPaths(base, head, paths, cwd) : [];
  return {paths, historyKnown, workflowControls, selected: select(paths, historyKnown, workflowControls)};
}

export function captureToolTests(cwd = root): Command[] {
  // Load tracked files explicitly: discovery skips hyphenated module filenames
  // and does not recurse into these non-package test folders.
  const result = spawnSync('git', ['ls-files', '-z', '--', 'capture/desktop/scripts/', 'capture/agent/scripts/'], { cwd, encoding: 'utf8' });
  if (result.error || result.status !== 0) throw Error('Cannot enumerate capture tooling tests');
  return result.stdout.split('\0').filter(path => /\/test[-_][^/]*\.py$/.test(path)).sort()
    .map(path => ({ argv: ['python3', '-m', 'unittest', path], cwd: '.',
      ...(path.endsWith('/test_issue_cli.py') ? {env: {APPARENT_BIN: resolve(cwd, process.env.CARGO_TARGET_DIR || 'target', 'debug/apparent')}} : {}) }));
}

export function commands(selected: Owner[], base: string, head: string, platform = process.platform): Command[] {
  const result: Command[] = [];
  const add = (argv: string[], cwd = '.') => result.push({ argv, cwd });
  // These cheap contracts were previously missing from feature pushes.
  add(['python3', 'enablement/verification.py', '--base', base, '--head', head, '--pr']);
  const tools = selected.includes('capture-tools') ? captureToolTests() : [];
  result.push(...tools.filter(command => !command.env?.APPARENT_BIN));
  if (selected.includes('tooling')) add(['python3', '-m', 'unittest', 'discover', '-s', 'enablement/directory-width/tests', '-p', 'test_*.py']);
  if (selected.includes('hermes')) add(['python3', '-m', 'unittest', 'discover', '-s', 'hermes/tests', '-p', 'test_*.py']);
  if (selected.includes('capture') || selected.includes('engine')) {
    // Reuse the authoritative conservative Rust verifier, including TMPDIR and jobs.
    // For mixed inputs it broadens to root + staged Engine rather than omit tests.
    add(['python3', 'enablement/verification.py', '--base', base, '--head', head, '--run']);
  }
  // This real CLI suite needs the exact executable the preceding root check
  // rebuilt. Its existing APPARENT_BIN contract supports private target dirs.
  if (tools.some(command => command.env?.APPARENT_BIN) && !selected.includes('capture')) throw Error('CLI tests require root build coverage');
  result.push(...tools.filter(command => command.env?.APPARENT_BIN));
  if (selected.includes('client')) {
    add(['python3', 'enablement/verification.py', '--base', base, '--preflight']);
    const cargo = platform === 'darwin' ? ['taskpolicy', '-c', 'utility', 'cargo'] : ['cargo'];
    for (const action of ['build', 'test']) add([...cargo, action, '--manifest-path', 'connect/client/Cargo.toml',
      '-j', process.env.T3_BUILD_JOBS || '3', '--offline', ...(action === 'test' ? ['--', '--test-threads=2'] : [])]);
  }
  if (selected.includes('desktop')) {
    const cwd = 'capture/desktop/ui/frontend';
    add(['npm', 'ci', '--no-audit', '--no-fund'], cwd);
    add(['npm', 'test'], cwd);
    add([...(platform === 'linux' ? ['xvfb-run', '-a'] : []), 'npm', 'run', 'test:electron'], cwd);
  }
  if (selected.includes('service')) {
    add(['npm', 'ci', '--no-audit', '--no-fund'], 'connect/service');
    add(['npm', 'test'], 'connect/service');
    add(['npm', 'run', 'test:recipient'], 'connect/service');
  }
  if (selected.includes('web')) {
    add(['bun', 'install', '--frozen-lockfile'], 'connect/web');
    add(['npm', 'test'], 'connect/web');
  }
  if (selected.includes('public')) add(['node', 'scripts/site-build/verify.mjs'], 'public/site');
  return result;
}

export function contractTests(cwd = root): Command[] {
  const prerequisites = readdirSync(resolve(cwd, 'enablement/prerequisites'))
    .filter(name => name.endsWith('.test.ts')).sort()
    .map(name => `enablement/prerequisites/${name}`);
  if (!prerequisites.length) throw Error('Native prerequisite contract inventory is empty');
  return [
    { argv: ['node', '--experimental-strip-types', '--test', 'enablement/local-tests/run.test.ts',
      ...prerequisites, 'enablement/timing/command.test.ts', 'enablement/timing/mac-cache.test.ts'], cwd: '.' },
    { argv: ['bun', 'test', 'enablement/ci/'], cwd: '.' },
  ];
}

export function execute(plan: Command[], cwd = root): void {
  // Git exports repository-local routing into hooks. Test fixtures and package
  // subprocesses must resolve their own cwd rather than mutate the caller's Git
  // config/index. Preserve identity/credentials and all non-local environment.
  const local = spawnSync('git', ['rev-parse', '--local-env-vars'], { cwd, encoding: 'utf8' });
  if (local.error || local.status !== 0) throw Error('Cannot isolate hook Git environment');
  const env = { ...process.env };
  for (const name of local.stdout.trim().split('\n')) delete env[name];
  for (const { argv, cwd: directory, env: extra } of plan) {
    console.error(`pre-push: [${directory}] ${argv.join(' ')}`);
    const result = spawnSync(argv[0], argv.slice(1), { cwd: resolve(cwd, directory), env: {...env, ...extra}, stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) throw Error(`Local test command failed (${result.signal || result.status}): ${argv.join(' ')}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === '--contracts') {
    execute(contractTests());
    process.exit(0);
  }
  const value = (flag: string) => args[args.indexOf(flag) + 1];
  if (!args.includes('--base') || !args.includes('--head')) throw Error('--base and --head are required');
  const base = value('--base'), head = value('--head');
  const result = selection(base, head, root, args.includes('--conservative-workflows'));
  const plan = commands(result.selected, base, head);
  console.log(JSON.stringify({ ...result, commands: plan }));
  if (args.includes('--run')) execute(plan);
}

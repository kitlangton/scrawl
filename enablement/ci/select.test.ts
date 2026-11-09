import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, readdirSync, mkdtempSync, rmSync, mkdirSync, writeFileSync, symlinkSync, unlinkSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {YAML} from 'bun';
import {select, changedPaths, requiresMac} from './select.ts';

const root = new URL('../../', import.meta.url);
const workflow = (name: string) => YAML.parse(readFileSync(new URL(`.github/workflows/${name}`, root), 'utf8')) as any;

test('only one automatic workflow; manual diagnostics and reporter retirement are preserved', () => {
  const automatic = readdirSync(new URL('.github/workflows/', root)).filter(name => {
    const events = Object.keys(workflow(name).on);
    return events.some(event => !['workflow_dispatch', 'workflow_call'].includes(event));
  });
  assert.deepEqual(automatic, ['checks.yml']);
  const ci = workflow('checks.yml');
  assert.deepEqual(Object.keys(ci.on), ['pull_request', 'push']);
  assert.deepEqual(ci.on.pull_request, {branches: ['main'],
    types: ['opened', 'reopened', 'synchronize', 'ready_for_review', 'converted_to_draft']});
  assert.deepEqual(ci.on.push, {branches: ['main']});
  assert.equal(ci.jobs['verification-status'], undefined);
  assert.equal(ci.on.workflow_run, undefined);
  assert.equal(ci.on.pull_request_target, undefined);
  assert.equal(readdirSync(new URL('enablement/', root)).includes('verification-status'), false);
  assert.equal(ci.permissions.contents, 'read');
  assert.equal(ci.jobs.public.uses, './.github/workflows/stream-public.yml');
  assert.equal(ci.jobs.public.if, "needs.select.outputs.public == 'true'");
  assert.equal(ci.jobs.updater.uses, './.github/workflows/linux-updater-harness.yml');
  assert.equal(ci.jobs.updater.if, "github.event_name == 'push' && needs.select.outputs.updater == 'true'");
  assert.match(workflow('stream-public.yml').jobs.deploy.if, /github.event_name == 'workflow_dispatch'/);
});

test('main preserves active checks while PR updates supersede them', () => {
  for (const filename of ['checks.yml', 'stream-public.yml']) {
    const concurrency = workflow(filename).concurrency;
    const expression = concurrency['cancel-in-progress'].replace(/^\$\{\{\s*|\s*\}\}$/g, '');
    for (const [event, expected] of [['push', false], ['pull_request', true], ['workflow_dispatch', false]]) {
      assert.equal(Function('github', `return (${expression});`)({event_name: event}), expected, `${filename}: ${event}`);
    }
    // GitHub's default single pending slot replaces older pending runs.
    assert.equal(concurrency.queue, undefined);
    assert.match(concurrency.group, /github.ref/);
  }
});

test('platform admission keeps main full and exempts only selected ready PRs from Mac', () => {
  const ci = workflow('checks.yml');
  const platform = workflow('verification-platform.yml');
  // Evaluate the workflow's actual boolean admission expressions against event
  // fixtures. These expressions use the common GitHub/JavaScript boolean subset.
  const admitted = (expression: string, event: string, draft: boolean, ref: string,
    receipt = false, macos = 'true') => Function('github', 'inputs', 'needs', 'cancelled', `return (${expression});`)(
      {event_name: event, ref, repository: 'tryapparent/apparent',
        event: {pull_request: {draft}}}, {'release_receipt': receipt},
      {contracts: {result: 'success'}, hermes: {result: 'success'},
        select: {result: 'success', outputs: {macos}}}, () => false);
  for (const [name, runner] of [['linux', 'ubuntu-24.04'], ['macos', 'macos-15-intel']]) {
    const job = ci.jobs[name];
    assert.equal(job.uses, './.github/workflows/verification-platform.yml');
    assert.deepEqual(job.with, {runner, 'release_receipt': false});
    assert.equal(job['continue-on-error'], undefined);
    for (const [event, draft, ref, expected] of [
      ['pull_request', false, 'refs/pull/7/merge', true],
      ['pull_request', true, 'refs/pull/7/merge', false],
      ['push', false, 'refs/heads/main', true],
      ['workflow_run', false, 'refs/heads/main', false],
      ['pull_request_target', false, 'refs/heads/main', false],
    ] as const) {
      assert.equal(admitted(job.if, event, draft, ref), expected);
      for (const selection of ['false', 'true', '']) {
        assert.equal(admitted(job.if, event, draft, ref, false, selection),
          expected && !(name === 'macos' && event === 'pull_request' && selection === 'false'));
      }
      if (expected) for (const child of ['rust', 'desktop-ui']) {
        assert.equal(admitted(platform.jobs[child].if, event, draft, ref), true, child);
        assert.equal(platform.jobs[child]['continue-on-error'], undefined);
        assert.equal(platform.jobs[child].steps.find((step: any) => step.uses === 'actions/checkout@v4').with.ref, undefined);
      }
      assert.equal(admitted(platform.jobs['release-ready'].if, event, draft, ref), false);
    }
  }
  assert.deepEqual(ci.jobs.macos.needs, ['contracts', 'hermes', 'select']);
  assert.deepEqual(ci.jobs.linux.needs, ['contracts', 'hermes', 'select']);
  assert.equal(ci.jobs.select.outputs.macos, '${{ steps.paths.outputs.macos }}');
  assert.equal(platform.on.workflow_call.inputs['release_receipt'].default, true);
  assert.equal(admitted(platform.jobs['release-ready'].if, 'workflow_dispatch', false, 'refs/heads/main', true), true);
  assert.equal(admitted(platform.jobs['release-ready'].if, 'pull_request', false, 'refs/pull/7/merge', true), false);
  for (const filename of ['verification.yml', 'verification-mac.yml']) {
    assert.deepEqual(Object.keys(workflow(filename).on), ['workflow_dispatch']);
  }
  assert.match(ci.concurrency.group, /github.event.pull_request.number/);
  assert.match(ci.concurrency['cancel-in-progress'], /pull_request/);
  assert.equal(ci.permissions.contents, 'read');
});

test('Public and updater retain their previous automatic input coverage', () => {
  for (const path of ['public/site/src/page.ts', 'shared/ui/button.ts', 'shared/analytics/index.ts',
    'shared/assets/logo.svg', '.github/workflows/stream-public.yml']) {
    assert.deepEqual(select([path]), {public: true, updater: false}, path);
  }
  for (const path of ['capture/desktop/scripts/linux/gui/gui-vm.py',
    'capture/desktop/scripts/linux/updater/updater-fixture.py', 'capture/desktop/linux/update.rs',
    'capture/desktop/settings/settings.rs', 'capture/desktop/settings/settings_window.rs',
    '.github/workflows/linux-updater-harness.yml']) {
    assert.deepEqual(select([path]), {public: false, updater: true}, path);
  }
  assert.deepEqual(select(['README.md']), {public: false, updater: false});
  assert.deepEqual(select([]), {public: false, updater: false});
  for (const paths of [null, ['.github/workflows/checks.yml'], ['enablement/ci/select.ts']]) {
    assert.deepEqual(select(paths), {public: true, updater: true});
  }
});

test('full platforms and Mac inspection share cheap prerequisites without depending on either full platform', () => {
  const ci = workflow('checks.yml');
  const expensive = Object.entries(ci.jobs).filter(([, job]: any) => {
    const runners = [job.with?.runner, job['runs-on'], ...Object.values(job.strategy?.matrix ?? {}).flat()];
    return job.uses === './.github/workflows/verification-platform.yml' ||
      runners.some(runner => typeof runner === 'string' && runner.startsWith('macos-'));
  });
  assert.deepEqual(expensive.map(([name]) => name).sort(), ['hermes-macos', 'linux', 'macos']);
  for (const cheap of ['contracts', 'hermes', 'select']) {
    assert.equal(ci.jobs[cheap].needs, undefined, `${cheap} must not wait for full platforms`);
    assert.equal(ci.jobs[cheap]['continue-on-error'], undefined);
  }
  for (const [name, job] of expensive.sort(([a], [b]) => a.localeCompare(b)) as [string, any][]) {
    assert.deepEqual([job.needs ?? []].flat().sort(), ['contracts', 'hermes', 'select'], name);
    assert.equal(job['continue-on-error'], undefined, name);
    // Evaluate the actual expression, including GitHub's implicit success()
    // only when no explicit status function overrides it. A failed selector
    // must retain full coverage, while failed cheap guards block allocation.
    const admitted = (needs: any, event = 'pull_request', draft = false, cancelled = false) => {
      const expression = job.if.replace(/^\$\{\{\s*|\s*\}\}$/g, '');
      const explicitStatus = /\b(always|success|failure|cancelled)\s*\(/.test(expression);
      if (!explicitStatus && job.needs.some((parent: string) => needs[parent]?.result !== 'success')) return false;
      return Function('needs', 'github', 'cancelled', `return (${expression});`)(needs,
        {event_name: event, event: {pull_request: {draft}}}, () => cancelled);
    };
    for (const contracts of ['success', 'failure', 'cancelled', 'skipped', 'unknown', undefined]) {
      for (const hermes of ['success', 'failure', 'cancelled', 'skipped', 'unknown', undefined]) {
        for (const select of ['success', 'failure', 'cancelled', 'skipped', 'unknown', undefined]) {
          for (const selected of ['true', 'false', 'unknown', undefined]) {
            const needs = {contracts: {result: contracts}, hermes: {result: hermes},
              select: {result: select, outputs: selected === undefined ? {} : {macos: selected}},
              linux: {result: 'failure'}, macos: {result: 'failure'}};
            const guardsPass = contracts === 'success' && hermes === 'success';
            for (const [event, draft, allowed] of [['pull_request', false, true], ['push', false, true],
              ['pull_request', true, false], ['workflow_run', false, false], ['workflow_dispatch', false, false]] as const) {
              const exempt = name === 'macos' && event === 'pull_request' && select === 'success' && selected === 'false';
              assert.equal(admitted(needs, event, draft), guardsPass && allowed && !exempt,
                `${name}: ${event}, draft=${draft}, guards=${contracts}/${hermes}, selector=${select}, selected=${selected}`);
            }
            assert.equal(admitted(needs, 'pull_request', false, true), false, `${name}: workflow cancellation`);
          }
        }
      }
    }
  }
  assert.equal(ci.jobs.hermes['runs-on'], 'ubuntu-24.04');
});

test('Mac inspection preserves its checks but runs only for ready PRs and main pushes', () => {
  const ci = workflow('checks.yml');
  const inspection = ci.jobs['hermes-macos'];
  assert.ok(inspection);
  assert.equal(inspection['runs-on'], 'macos-15-intel');
  assert.deepEqual(inspection.steps, ci.jobs.hermes.steps);
  // The Settings exemption applies to the platform workflow; Hermes keeps
  // main's own event admission and cheap-check barrier.
  assert.equal(inspection.if, ci.jobs.linux.if);
});

test('unknown history runs all checks and git deltas include deleted and renamed owner paths', () => {
  assert.equal(changedPaths('', ''), null);
  assert.equal(changedPaths('0'.repeat(40), 'a'.repeat(40)), null);
  const dir = mkdtempSync(`${process.env.TMPDIR ?? '/tmp'}/apparent-dev-ci-fixture-`);
  const previous = process.cwd();
  try {
    process.chdir(dir);
    const git = (...args: string[]) => execFileSync('git', args, {encoding: 'utf8'}).trim();
    git('init', '-q');
    git('config', 'user.name', 'CI fixture');
    git('config', 'user.email', 'ci@example.invalid');
    // Synthetic commits only; no repository hooks or user identity changes.
    mkdirSync('public/site', {recursive: true});
    writeFileSync('public/site/old.txt', 'fixture');
    git('add', '.'); git('commit', '-qm', 'before');
    const base = git('rev-parse', 'HEAD');
    git('mv', 'public/site/old.txt', 'outside.txt');
    git('commit', '-qm', 'after');
    const paths = changedPaths(base, git('rev-parse', 'HEAD'))!;
    assert.deepEqual(paths.sort(), ['outside.txt', 'public/site/old.txt']);
    assert.equal(select(paths).public, true);
    assert.deepEqual(changedPaths(base, git('rev-parse', 'HEAD'), true), ['outside.txt', 'public/site/old.txt']);
    assert.equal(requiresMac(paths, changedPaths(base, git('rev-parse', 'HEAD'), true)), true);
    assert.equal(changedPaths('a'.repeat(40), git('rev-parse', 'HEAD')), null);
    const card = 'capture/desktop/ui/frontend/src/storage.tsx';
    mkdirSync('capture/desktop/ui/frontend/src', {recursive: true});
    const runSelector = (from: string) => {
      const output = dir + '/selection.txt';
      writeFileSync(output, '');
      execFileSync(process.execPath, [new URL('enablement/ci/select.ts', root).pathname], {
        env: {...process.env, BASE_SHA: from, GITHUB_SHA: git('rev-parse', 'HEAD'), GITHUB_OUTPUT: output},
      });
      return Object.fromEntries(readFileSync(output, 'utf8').trim().split('\n').map(line => line.split('=')));
    };
    const beforeAdd = git('rev-parse', 'HEAD');
    writeFileSync(card, 'before');
    git('add', card); git('commit', '-qm', 'add card');
    assert.equal(runSelector(beforeAdd).macos, 'true');
    const beforeEdit = git('rev-parse', 'HEAD');
    writeFileSync(card, 'after');
    git('add', card); git('commit', '-qm', 'edit card');
    assert.deepEqual(runSelector(beforeEdit), {public: 'false', updater: 'false', macos: 'false'});
    const beforeType = git('rev-parse', 'HEAD');
    unlinkSync(card); symlinkSync('retention.tsx', card);
    git('add', card); git('commit', '-qm', 'change card type');
    assert.equal(runSelector(beforeType).macos, 'true');
    const beforeDelete = git('rev-parse', 'HEAD');
    git('rm', card); git('commit', '-qm', 'remove card');
    assert.equal(runSelector(beforeDelete).macos, 'true');

  } finally {
    process.chdir(previous);
    rmSync(dir, {recursive: true, force: true});
  }
});

test('Mac exemption is exact, nonempty, modification-only and fails closed', () => {
  const prefix = 'capture/desktop/ui/frontend/';
  const allowed = ['storage', 'retention', 'connect-settings', 'error-reporting', 'reference']
    .map(name => prefix + 'src/' + name + '.tsx');
  assert.equal(requiresMac(allowed), false);
  for (const path of allowed) {
    assert.equal(requiresMac([path]), false);
    assert.equal(requiresMac([path], [path]), true);
  }
  for (const path of ['src/new.tsx', 'src/style.css', 'src/settings.tsx', 'src/bridge.ts',
    'src/notes/index.tsx', 'src/onboarding/onboarding.tsx', 'electron/main.cjs',
    'electron/preload.cjs', 'package-lock.json', 'tests/storage-smoke.cjs']) {
    assert.equal(requiresMac([...allowed, prefix + path]), true, path);
  }
  for (const path of ['shared/ui/src/button.tsx', 'capture/desktop/mod.rs',
    '.github/workflows/checks.yml', 'enablement/ci/select.ts', 'README.md']) {
    assert.equal(requiresMac([...allowed, path]), true, path);
  }
  assert.equal(requiresMac(null), true);
  assert.equal(requiresMac([]), true);
  assert.equal(requiresMac(allowed, null), true);
});

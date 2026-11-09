import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, readdirSync, mkdtempSync, rmSync, mkdirSync, writeFileSync, symlinkSync, unlinkSync, chmodSync, existsSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {YAML} from 'bun';
import {select, changedPaths, requiresMac, macDecision, selectRange, documentationOnlyPath, needsPlatform} from './select.ts';
import {changedPaths as sharedInputPaths, workflowControlPaths} from './inputs.ts';

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
    assert.equal(concurrency.queue, undefined);
    assert.match(concurrency.group, filename === 'checks.yml' ? /github.sha/ : /github.ref/);
    const group = (sha: string, pr?: number) => concurrency.group.replace(/\$\{\{(.+?)\}\}/g,
      (_: string, expression: string) => Function('github', 'return (' + expression + ');')({
        event_name: pr ? 'pull_request' : 'push', sha, ref: 'refs/heads/main',
        event: {pull_request: {number: pr}},
      }));
    if (filename === 'checks.yml') assert.notEqual(group('code-sha'), group('docs-sha'));
    else assert.equal(group('code-sha'), group('docs-sha'));
    assert.equal(group('code-sha', 7), group('docs-sha', 7));
  }
});

test('platform admission composes ordinary docs with selected ready-PR Mac exemptions', () => {
  const ci = workflow('checks.yml');
  const platform = workflow('verification-platform.yml');
  // Evaluate the workflow's actual boolean admission expressions against event
  // fixtures. These expressions use the common GitHub/JavaScript boolean subset.
  const admitted = (expression: string, event: string, draft: boolean, ref: string,
    receipt = false, macos = 'true', result = 'success', cancelled = false, platform = 'true') => Function('github', 'inputs', 'needs', 'cancelled',
      `return (${expression.replace(/^\$\{\{\s*|\s*\}\}$/g, '')});`)(
      {event_name: event, ref, repository: 'tryapparent/apparent',
        event: {pull_request: {draft}}}, {'release_receipt': receipt}, {
        linux: {result: 'failure'}, contracts: {result: 'success'}, hermes: {result: 'success'},
        select: {result, outputs: {macos, platform}}}, () => cancelled);
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
  assert.equal(ci.jobs.select.outputs.platform, '${{ steps.paths.outputs.platform }}');
  const selector = ci.jobs.select.steps.find((step: any) => step.id === 'paths');
  assert.equal(selector.env.BASE_SHA, '${{ github.event.pull_request.base.sha || github.event.before }}');
  assert.equal(selector.run, 'bun enablement/ci/select.ts');
  assert.equal(selector['continue-on-error'], undefined);
  assert.equal(ci.jobs.select.steps.find((step: any) => step.uses === 'actions/checkout@v4').with['fetch-depth'], 0);
  // GitHub adds success() unless the expression contains a status function.
  // Evaluate the actual YAML with that dependency-skip behavior too.
  const macAdmitted = (event: string, result: string, output: string, cancelled = false) =>
    (/\b(always|cancelled|failure|success)\(/.test(ci.jobs.macos.if) || result === 'success') &&
      admitted(ci.jobs.macos.if, event, false, 'refs/heads/main', false, output, result, cancelled);
  for (const result of ['success', 'failure', 'skipped', 'cancelled']) {
    for (const output of ['false', 'true', '', 'garbage']) {
      assert.equal(macAdmitted('pull_request', result, output), result !== 'success' || output !== 'false');
      assert.equal(macAdmitted('push', result, output), true);
      assert.equal(macAdmitted('pull_request', result, output, true), false);
    }
  }
  assert.equal(platform.on.workflow_call.inputs['release_receipt'].default, true);
  assert.equal(admitted(platform.jobs['release-ready'].if, 'workflow_dispatch', false, 'refs/heads/main', true), true);
  assert.equal(admitted(platform.jobs['release-ready'].if, 'pull_request', false, 'refs/pull/7/merge', true), false);
  for (const filename of ['verification.yml', 'verification-mac.yml']) {
    assert.deepEqual(Object.keys(workflow(filename).on), ['workflow_dispatch']);
    assert.equal(workflow(filename).jobs.verify.if, undefined);
    assert.equal(workflow(filename).jobs.verify.uses, './.github/workflows/verification-platform.yml');
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
  assert.deepEqual(select(['README.md']), {public: true, updater: false});
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
    const results = ['success', 'failure', 'cancelled', 'skipped', undefined, 'unknown'];
    for (const contracts of results) {
      for (const hermes of results) {
        for (const select of results) {
          for (const output of ['true', 'false', '', undefined, 'unknown']) {
            for (const platform of ['true', 'false', '', undefined, 'unknown']) {
              const needs = {contracts: {result: contracts}, hermes: {result: hermes},
                select: {result: select, outputs: {macos: output, platform}}, linux: {result: 'failure'}, macos: {result: 'failure'}};
              const expected = contracts === 'success' && hermes === 'success';
              const docsExempt = select === 'success' && platform === 'false';
              for (const [event, draft, allowed] of [['pull_request', false, true], ['push', false, true],
                ['pull_request', true, false], ['workflow_run', false, false], ['workflow_dispatch', false, false]] as const) {
                const exempt = name === 'macos' && event === 'pull_request' && select === 'success' && output === 'false';
                assert.equal(admitted(needs, event, draft), expected && allowed && !exempt && !docsExempt,
                  name + ': ' + [event, draft, contracts, hermes, select, output, platform].join('/'));
              }
              assert.equal(admitted(needs, 'pull_request', false, true), false, name + ': workflow cancellation');
            }
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
  // main's own event admission and cheap shared prerequisite guards.
  assert.equal(inspection.if, ci.jobs.linux.if);
});

// Isolate the entire synchronous scope: production helpers also spawn Git.
// cwd alone cannot protect the caller's index/config from inherited hook vars.
function repository(action: (git: (...args: string[]) => string, dir: string) => void) {
  const dir = mkdtempSync(`${process.env.TMPDIR ?? '/tmp'}/apparent-dev-ci-fixture-`);
  const previous = process.cwd();
  const inherited: Record<string, string> = {};
  for (const key of Object.keys(process.env)) if (key.startsWith('GIT_')) {
    inherited[key] = process.env[key]!;
    delete process.env[key];
  }
  try {
    process.chdir(dir);
    const git = (...args: string[]) => execFileSync('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', ...args],
      {encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe']}).trim();
    git('init', '-qb', 'main');
    git('config', 'user.name', 'CI fixture');
    git('config', 'user.email', 'ci@example.invalid');
    action(git, dir);
  } finally {
    process.chdir(previous);
    Object.assign(process.env, inherited);
    rmSync(dir, {recursive: true, force: true});
  }
}

function runSelector(base: string, head: string, dir: string) {
  const output = dir + '/.git/selection.txt';
  writeFileSync(output, '');
  const stdout = execFileSync(process.execPath, [new URL('enablement/ci/select.ts', root).pathname], {
    env: {...process.env, BASE_SHA: base, GITHUB_SHA: head, GITHUB_OUTPUT: output}, encoding: 'utf8',
  });
  const decision = JSON.parse(stdout);
  const outputs = Object.fromEntries(readFileSync(output, 'utf8').trim().split('\n').map(line => line.split('=')));
  assert.deepEqual(outputs, {public: String(decision.public), updater: String(decision.updater), macos: String(decision.macos), platform: String(decision.platform)});
  return decision as ReturnType<typeof selectRange>;
}

test('unknown history runs all checks and git deltas include deleted and renamed owner paths', () => repository((git, dir) => {
  assert.equal(changedPaths('', ''), null);
  assert.equal(changedPaths('0'.repeat(40), 'a'.repeat(40)), null);
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
    const beforeAdd = git('rev-parse', 'HEAD');
    writeFileSync(card, 'before');
    git('add', card); git('commit', '-qm', 'add card');
    assert.equal(runSelector(beforeAdd, git('rev-parse', 'HEAD'), dir).macos, true);
    const beforeEdit = git('rev-parse', 'HEAD');
    writeFileSync(card, 'after');
    git('add', card); git('commit', '-qm', 'edit card');
    assert.deepEqual(runSelector(beforeEdit, git('rev-parse', 'HEAD'), dir),
      {public: false, updater: false, macos: false, platform: true, reasons: [`linux:reviewed-settings:${card}`]});
    const beforeType = git('rev-parse', 'HEAD');
    unlinkSync(card); symlinkSync('retention.tsx', card);
    git('add', card); git('commit', '-qm', 'change card type');
    assert.equal(runSelector(beforeType, git('rev-parse', 'HEAD'), dir).macos, true);
    const beforeDelete = git('rev-parse', 'HEAD');
    git('rm', card); git('commit', '-qm', 'remove card');
    assert.equal(runSelector(beforeDelete, git('rev-parse', 'HEAD'), dir).macos, true);
}));

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

const portal = 'capture/desktop/linux/capture/portal.rs';
const linuxDoc = 'capture/desktop/docs/linux/linux.md';
const retiredFeatureMap = 'public/site/src/features.json';

function linuxFixture(action: (git: (...args: string[]) => string, dir: string, base: string) => void) {
  repository((git, dir) => {
    for (const path of [portal, linuxDoc, 'capture/desktop/ui/frontend/src/storage.tsx']) {
      mkdirSync(path.slice(0, path.lastIndexOf('/')), {recursive: true});
      writeFileSync(path, readFileSync(new URL(path, root)));
    }
    git('add', '.'); git('commit', '-qm', 'audited inputs');
    action(git, dir, git('rev-parse', 'HEAD'));
  });
}

test('production Git/CLI selects full Linux for the portal and guide after catalogue retirement', () => linuxFixture((git, dir, base) => {
  // The surviving current-main inputs of the original portal source-type fix.
  // No PR, commit or English-prose matching participates in selection.
  appendFile(portal, '\nfn selected_source_type(source_type: Option<u32>) -> bool { source_type != Some(2) }\n');
  appendFile(linuxDoc, '\nA documented portal scope restriction.\n');
  git('add', '.'); git('commit', '-qm', 'portal restriction and documentation');
  const head = git('rev-parse', 'HEAD');
  assert.deepEqual(changedPaths(base, head), [linuxDoc, portal]);
  assert.deepEqual(changedPaths(base, head), sharedInputPaths(base, head, process.cwd()));
  const expected = {public: false, updater: false, macos: false, platform: true, reasons: [
    'linux:reviewed-portal-input:' + linuxDoc, 'linux:reviewed-portal-input:' + portal,
  ]};
  assert.deepEqual(selectRange(base, head), expected);
  assert.deepEqual(runSelector(base, head, dir), expected);
  writeFileSync(portal, 'Later uncommitted contents');
  assert.deepEqual(runSelector(base, head, dir), expected);
}));

function appendFile(path: string, text: string) {
  writeFileSync(path, readFileSync(path, 'utf8') + text);
}

test('retired catalogue and unknown metadata retain Mac and Public even for a note-only modification', () => linuxFixture((git, dir) => {
  mkdirSync('public/site/src', {recursive: true});
  writeFileSync(retiredFeatureMap, '{"note":"before"}');
  git('add', '.'); git('commit', '-qm', 'unknown metadata baseline');
  const base = git('rev-parse', 'HEAD');
  writeFileSync(retiredFeatureMap, '{"note":"after"}');
  appendFile(portal, '\n// Linux source change\n');
  git('add', '.'); git('commit', '-qm', 'mixed unknown note and Linux source');
  const result = runSelector(base, git('rev-parse', 'HEAD'), dir);
  assert.equal(result.platform, true);
  assert.equal(result.public, true);
  assert.equal(result.macos, true);
  assert.ok(result.reasons.includes('mac:unproven-input:' + retiredFeatureMap));
}));

test('proved concurrency-only local input never exempts hosted Mac verification', () => repository((git, dir) => {
  const path = '.github/workflows/checks.yml';
  mkdirSync('.github/workflows', {recursive: true});
  const before = readFileSync(new URL(path, root), 'utf8');
  writeFileSync(path, before); git('add', '.'); git('commit', '-qm', 'orchestrator base');
  const base = git('rev-parse', 'HEAD');
  writeFileSync(path, before.replace(/  cancel-in-progress: .*/, '  cancel-in-progress: false'));
  git('add', '.'); git('commit', '-qm', 'concurrency control');
  const head = git('rev-parse', 'HEAD');
  assert.deepEqual(workflowControlPaths(base, head, sharedInputPaths(base, head, dir), dir), [path]);
  assert.deepEqual(runSelector(base, head, dir), {public: true, updater: true, macos: true, platform: true,
    reasons: [`mac:unproven-input:${path}`]});
}));

test('Linux portal and guide compose with Settings while ordinary docs retain their existing gate', () => linuxFixture((git, dir, base) => {
  for (const path of [portal, linuxDoc, 'capture/desktop/ui/frontend/src/storage.tsx']) {
    git('checkout', '--detach', base);
    appendFile(path, '\n// synthetic edit\n');
    git('add', '.'); git('commit', '-qm', 'reviewed modification');
    const result = runSelector(base, git('rev-parse', 'HEAD'), dir);
    assert.equal(result.macos, false, path);
    assert.equal(result.public, false, path);
    assert.equal(result.platform, path !== linuxDoc, path);
  }
  appendFile(portal, '\n// portal edit\n');
  appendFile(linuxDoc, '\nDocumentation for this portal edit.\n');
  git('add', '.'); git('commit', '-qm', 'compose portal, guide and Settings');
  const result = runSelector(base, git('rev-parse', 'HEAD'), dir);
  assert.equal(result.macos, false);
  assert.equal(result.platform, true);
}));

test('mixed shared, Mac, ABI, dependencies, packaging, CI and unknown inputs retain Mac', () => linuxFixture((git, dir, base) => {
  for (const path of ['shared/model.rs', 'capture/desktop/native/Settings.swift', 'capture/desktop/mod.rs',
    'capture/desktop/linux/capture/mod.rs', 'capture/desktop/ui/frontend/src/bridge.ts',
    'build.rs', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'capture/desktop/ui/frontend/package-lock.json',
    'capture/desktop/scripts/packaging/build-mac-preview.sh', '.github/workflows/checks.yml',
    'capture/engine/docs/other-owner.md', 'public/site/src/runtime-metadata.json', retiredFeatureMap, 'unknown.md']) {
    git('checkout', '--detach', base);
    appendFile(portal, '\n// portal edit\n');
    mkdirSync(path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '.', {recursive: true});
    writeFileSync(path, 'Unreviewed input');
    git('add', '.'); git('commit', '-qm', 'mixed delta');
    const result = runSelector(base, git('rev-parse', 'HEAD'), dir);
    assert.equal(result.macos, true, path);
    assert.equal(result.platform, true, path);
    assert.ok(result.reasons.includes('mac:unsupported-change:' + path));
    const modificationBase = git('rev-parse', 'HEAD');
    appendFile(path, '\nModified existing unreviewed input\n');
    appendFile(portal, '\n// another portal edit\n');
    git('add', '.'); git('commit', '-qm', 'mixed modifications');
    const modified = runSelector(modificationBase, git('rev-parse', 'HEAD'), dir);
    assert.equal(modified.macos, true, path);
    assert.equal(modified.platform, true, path);
    assert.ok(modified.reasons.includes('mac:unproven-input:' + path));
  }
}));

test('real Git additions, deletions, both rename sides, executable modes and symlinks fail closed', () => linuxFixture((git, dir, base) => {
  const card = 'capture/desktop/ui/frontend/src/storage.tsx';
  for (const path of [portal, linuxDoc, card]) {
    for (const operation of ['add', 'delete', 'rename-out', 'rename-in', 'chmod', 'symlink', 'edit-symlink']) {
      git('checkout', '--detach', base);
      if (operation === 'add' || operation === 'rename-in') {
        git('mv', path, 'outside.txt'); git('commit', '-qam', 'before addition or move');
      } else if (operation === 'edit-symlink') {
        unlinkSync(path); symlinkSync('first-target', path);
        git('add', '.'); git('commit', '-qm', 'symlink base');
      }
      const from = git('rev-parse', 'HEAD');
      if (operation === 'add') writeFileSync(path, 'new input');
      if (operation === 'delete') git('rm', path);
      if (operation === 'rename-out') git('mv', path, 'outside.txt');
      if (operation === 'rename-in') git('mv', 'outside.txt', path);
      if (operation === 'chmod') chmodSync(path, 0o755);
      if (operation === 'symlink' || operation === 'edit-symlink') { unlinkSync(path); symlinkSync('second-target', path); }
      git('add', '.'); git('commit', '-qm', 'unsupported change');
      const head = git('rev-parse', 'HEAD');
      const result = runSelector(from, head, dir);
      assert.equal(result.macos, true, `${path}: ${operation}`);
      assert.equal(result.platform, !(path === linuxDoc && ['add', 'delete'].includes(operation)), `${path}: ${operation}`);
      assert.ok(changedPaths(from, head, true)!.includes(path), `${path}: ${operation}`);
      if (operation.startsWith('rename-')) assert.deepEqual(changedPaths(from, head)!.sort(), [path, 'outside.txt'].sort());
    }
  }
}));

test('full merge delta carries earlier unverified changes; missing, divergent and shallow history retain coverage', () => linuxFixture((git, dir, base) => {
  writeFileSync('shared-contract.rs', 'earlier unverified change');
  git('add', '.'); git('commit', '-qm', 'earlier shared change');
  const earlier = git('rev-parse', 'HEAD');
  appendFile(linuxDoc, '\nLater documentation\n'); git('add', '.'); git('commit', '-qm', 'later docs');
  const head = git('rev-parse', 'HEAD');
  assert.equal(runSelector(earlier, head, dir).macos, false, 'latest-commit comparison would lose coverage');
  git('checkout', '-qb', 'base-advance', base);
  writeFileSync('base-only.txt', 'new main'); git('add', '.'); git('commit', '-qm', 'advance main');
  const currentBase = git('rev-parse', 'HEAD');
  git('merge', '--no-ff', '-qm', 'proposed PR merge', head);
  const merge = git('rev-parse', 'HEAD');
  const result = runSelector(currentBase, merge, dir);
  assert.equal(result.macos, true);
  assert.ok(result.reasons.includes('mac:unsupported-change:shared-contract.rs'));
  for (const [from, to] of [['', ''], ['0'.repeat(40), head], ['a'.repeat(40), head], [currentBase, head], [base, base]]) {
    const decision = runSelector(from, to, dir);
    assert.equal(decision.macos, true);
    if (from !== to || !from) assert.deepEqual(selectRange(from, to),
      {public: true, updater: true, macos: true, platform: true, reasons: ['mac:unknown-history']});
  }
  git('clone', '--quiet', '--no-local', '--depth=2', dir, dir + '/shallow');
  process.chdir(dir + '/shallow');
  assert.deepEqual(runSelector(currentBase, merge, dir + '/shallow'),
    {public: true, updater: true, macos: true, platform: true, reasons: ['mac:unknown-history']});
}));

test('audited consumers keep Linux commands, portable contracts, catalogue retirement and Mac release admission', () => {
  const source = (path: string) => readFileSync(new URL(path, root), 'utf8');
  assert.match(source('capture/desktop/mod.rs'), /#\[cfg\(target_os = "linux"\)\]\s*pub\(crate\) mod linux;/);
  assert.match(source('capture/desktop/linux/mod.rs'), /pub\(crate\) mod capture;/);
  assert.match(source('capture/desktop/linux/capture/mod.rs'), /pub\(crate\) mod portal;/);
  // Catalogue retirement removed the former Rust scanner and Public guide read.
  for (const path of [retiredFeatureMap, 'composition/feature_map.rs', 'public/site/src/features.ts',
    'public/site/scripts/site-build/feature-audit.ts']) assert.equal(existsSync(new URL(path, root)), false, path);
  assert.deepEqual(select([retiredFeatureMap]), {public: true, updater: false});
  assert.equal(requiresMac([retiredFeatureMap]), true);
  assert.deepEqual(select([linuxDoc]), {public: false, updater: false});
  const platform = workflow('verification-platform.yml');
  const steps = platform.jobs.select.steps;
  assert.match(steps.find((step: any) => step.id === 'plan').run, /git hash-object -t tree \/dev\/null/);
  assert.ok(steps.some((step: any) => step.run?.includes('--execution portable') && !step.if));
  assert.equal(platform.jobs.rust.needs, 'select');
  assert.equal(platform.jobs['desktop-ui'].needs, 'select');
  assert.ok(platform.jobs.rust.steps.some((step: any) => step.run?.includes('--execution platform')));
  assert.ok(platform.jobs['desktop-ui'].steps.some((step: any) => step.run === 'npm test' && !step.if));
  assert.ok(platform.jobs['desktop-ui'].steps.some((step: any) => step.run?.includes('xvfb-run -a npm run test:electron') && !step.if));
  assert.match(source('.github/workflows/release-mac-preview.yml'), /enablement\/release_gate.py --platform macos --sha "\$GITHUB_SHA"/);
  assert.equal(macDecision([portal, linuxDoc]).macos, false);
  assert.equal(macDecision([portal], ['unknown-input']).macos, true);
});

test('ordinary prose skips heavy CI; instructions, shipped text and unknown inputs retain coverage', () => {
  for (const path of ['capture/desktop/docs/linux/linux.md', 'enablement/docs/continuous-delivery.md',
    'public/site/docs/feature-grid.md', 'capture/engine/README.md', 'enablement/refactor.md']) {
    assert.equal(documentationOnlyPath(path), true, path);
    assert.equal(needsPlatform([path]), false, path);
    assert.deepEqual(select([path]), {public: false, updater: false}, path);
  }
  for (const path of ['AGENTS.md', 'capture/AGENTS.md', 'README.md', 'capture/agent/skills/apparent/SKILL.md',
    'hermes/skills/recordings/SKILL.md', 'capture/agent/instructions/context.md',
    'public/site/src/copy.md', 'capture/engine/tests/fixtures/summary.md', 'unknown.md',
    'public/site/src/docs/copy.md', 'public/site/public/assets/docs/help.md',
    'unknown/docs/input.md', 'unknown/README.md', 'public/site/src/README.md']) {
    assert.equal(needsPlatform([path]), true, path);
  }
  assert.equal(needsPlatform(null), true);
  assert.equal(needsPlatform([]), true);
  assert.equal(needsPlatform(['capture/engine/docs/capture.md', 'capture/engine/runtime/mod.rs']), true);
  assert.equal(select(['public/site/docs/guide.md', 'public/site/src/main.tsx']).public, true);
});

test('production CLI preserves regular prose additions, deletions and rename sides but rejects unsafe modes', () => repository((git, dir) => {
  const path = 'capture/engine/docs/guide.md';
  mkdirSync('capture/engine/docs', {recursive: true});
  writeFileSync('baseline.txt', 'baseline'); git('add', '.'); git('commit', '-qm', 'base');
  let base = git('rev-parse', 'HEAD');
  writeFileSync(path, 'Ordinary prose');
  git('add', '.'); git('commit', '-qm', 'add guide');
  let result = runSelector(base, git('rev-parse', 'HEAD'), dir);
  assert.equal(result.platform, false);
  assert.deepEqual(result.reasons, ['docs:reviewed-prose:' + path]);
  base = git('rev-parse', 'HEAD');
  git('mv', path, 'capture/engine/docs/renamed.md'); git('commit', '-qm', 'rename guide');
  assert.equal(runSelector(base, git('rev-parse', 'HEAD'), dir).platform, false);
  base = git('rev-parse', 'HEAD');
  git('rm', 'capture/engine/docs/renamed.md'); git('commit', '-qm', 'delete guide');
  assert.equal(runSelector(base, git('rev-parse', 'HEAD'), dir).platform, false);
  const empty = git('rev-parse', 'HEAD');
  for (const kind of ['executable', 'symlink']) {
    git('checkout', '--detach', empty);
    mkdirSync('capture/engine/docs', {recursive: true});
    if (kind === 'symlink') symlinkSync('../../../build.rs', path);
    else { writeFileSync(path, 'Unproven executable'); chmodSync(path, 0o755); }
    git('add', '.'); git('commit', '-qm', 'unsafe prose mode');
    result = runSelector(empty, git('rev-parse', 'HEAD'), dir);
    assert.equal(result.platform, true, kind);
    assert.equal(result.macos, true, kind);
    assert.equal(result.public, true, kind);
    assert.equal(result.updater, true, kind);
  }
}));

test('ordinary prose mixed with Settings cannot broaden the reviewed Mac exemption', () => linuxFixture((git, dir) => {
  const guide = 'capture/engine/docs/guide.md';
  mkdirSync('capture/engine/docs', {recursive: true}); writeFileSync(guide, 'before');
  git('add', '.'); git('commit', '-qm', 'prose baseline');
  const base = git('rev-parse', 'HEAD');
  appendFile(guide, '\nafter\n'); appendFile('capture/desktop/ui/frontend/src/storage.tsx', '\n// edit\n');
  git('add', '.'); git('commit', '-qm', 'mixed Settings and prose');
  const result = runSelector(base, git('rev-parse', 'HEAD'), dir);
  assert.equal(result.macos, true);
  assert.equal(result.platform, true);
  assert.equal(result.public, false);
}));

function publicMarkdownInputs() {
  const consumer = readFileSync(new URL('public/site/scripts/site-build/agent-files.ts', root), 'utf8');
  const paths = [...new Set([...consumer.matchAll(/\b(?:repo|guide)\("([^"]+\.md)"\)/g)].map(match => match[1]))].sort();
  assert.equal(paths.length, 5, 'Review the Public Markdown input boundary when its consumer changes');
  assert.ok(paths.every(path => !path.startsWith('/') && !path.split('/').includes('..')));
  return paths;
}

test('Public Markdown body consumers retain Public and full platform verification', () => {
  const paths = publicMarkdownInputs();
  assert.deepEqual(paths.map(path => ({path, prose: documentationOnlyPath(path),
    platform: needsPlatform([path]), ...select([path])})),
    paths.map(path => ({path, prose: false, platform: true, public: true, updater: false})));
});

test('production Git/CLI cannot exempt Public build Markdown alone or mixed with portal source', () => linuxFixture((git, dir) => {
  const paths = publicMarkdownInputs();
  for (const path of paths) {
    mkdirSync(path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '.', {recursive: true});
    writeFileSync(path, readFileSync(new URL(path, root)));
  }
  git('add', '.'); git('commit', '-qm', 'Public documentation baseline');
  const base = git('rev-parse', 'HEAD');
  for (const path of paths) {
    git('checkout', '--detach', base);
    appendFile(path, '\nArbitrary documentation edit.\n');
    git('add', '.'); git('commit', '-qm', 'Public build input');
    const result = runSelector(base, git('rev-parse', 'HEAD'), dir);
    assert.deepEqual({public: result.public, updater: result.updater,
      macos: result.macos, platform: result.platform},
      {public: true, updater: false, macos: true, platform: true}, path);
    appendFile(portal, '\n// mixed Linux source edit\n');
    git('add', '.'); git('commit', '-qm', 'Public input and portal source');
    const mixed = runSelector(base, git('rev-parse', 'HEAD'), dir);
    assert.equal(mixed.public, true, path);
    assert.equal(mixed.macos, true, path);
    assert.equal(mixed.platform, true, path);
  }
}));

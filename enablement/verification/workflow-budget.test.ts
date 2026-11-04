import assert from 'node:assert/strict'
import test from 'node:test'
import { executionSeconds, enforceBudget, workflowBudgetSeconds, loadJobs, auditName, type Job } from './workflow-budget.ts'
import { readFileSync, readdirSync, mkdtempSync, writeFileSync, rmSync, mkdirSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const job = (start: number, end: number, name = 'work'): Job => ({ name, status: 'completed', conclusion: 'success', started_at: new Date(start * 1000).toISOString(), completed_at: new Date(end * 1000).toISOString() })

test('union counts parallel execution once and excludes queue gaps', () => {
  assert.equal(executionSeconds([job(0, 100), job(30, 80), job(90, 200), job(500, 650)]), 350)
})
test('sequential reusable jobs share one workflow budget', () => {
  assert.throws(() => enforceBudget([job(0, 350), job(400, 700)]), /agent whose run exceeded the limit owns optimization/)
  assert.doesNotThrow(() => enforceBudget([job(0, 600)]))
  assert.equal(executionSeconds([job(0, 350, 'verify / rust'), job(400, 700, 'publish')]), 650)
  assert.equal(executionSeconds([job(0, 600)]), 600)
})
test('skip placeholders and audits do not consume the workload budget', () => {
  assert.equal(executionSeconds([{ ...job(10, 0), conclusion: 'skipped' },
    { ...job(0, 999), name: `runtime-budget / ${auditName}`, status: 'in_progress' },
    job(0, 20)]), 20)
})
test('failed and cancelled execution still counts; uncertain timestamps fail closed', () => {
  assert.equal(executionSeconds([{ ...job(0, 30), conclusion: 'failure' }, { ...job(30, 60), conclusion: 'cancelled' }]), 60)
  for (const entry of [{ ...job(0, 10), status: 'queued' }, { ...job(0, 10), started_at: null }, job(20, 10)])
    assert.throws(() => executionSeconds([entry]))
})
test('read all pages from exact run attempt, without redirecting credentials', async () => {
  const urls: string[] = []
  const request = async (url: string | URL | Request, options?: RequestInit) => {
    urls.push(String(url))
    assert.equal(options?.redirect, 'error')
    return new Response(JSON.stringify({ total_count: 2, jobs: [job(urls.length * 10, urls.length * 10 + 5)] }))
  }
  const jobs = await loadJobs('https://api.github.com', 'owner/repo', '42', '3', 'synthetic-token', request as typeof fetch)
  assert.equal(jobs.length, 2)
  assert.match(urls[0], /runs\/42\/attempts\/3\/jobs\?per_page=100&page=1$/)
  assert.match(urls[1], /page=2$/)
})
test('API denial and incomplete inventories cannot pass', async () => {
  for (const response of [new Response('', { status: 403 }), new Response(JSON.stringify({total_count: 1, jobs: []})), new Response('{}'), new Response(JSON.stringify({total_count: 0, jobs: []})), new Response(JSON.stringify({total_count: 1, jobs: [job(0, 1), job(1, 2)]}))])
    await assert.rejects(loadJobs('https://api.github.com', 'owner/repo', '42', '1', 'token', (async () => response) as typeof fetch))
})
test('every supported root workflow waits for all payload jobs before auditing', () => {
  const folder = new URL('../../.github/workflows/', import.meta.url)
  for (const name of readdirSync(folder)) {
    const text = readFileSync(new URL(name, folder), 'utf8')
    if (name === 'release-windows.yml' || name === 'runtime-budget.yml') continue
    // Only root workflows: callable platform verification is covered by caller.
    if (!/^  (push|workflow_dispatch|workflow_run|schedule):/m.test(text)) continue
    const jobs = [...text.split('jobs:\n')[1].matchAll(/^  ([\w-]+):$/gm)].map(m => m[1])
    assert.ok(jobs.includes('runtime-budget'), name)
    const gate = text.split('  runtime-budget:\n')[1]
    assert.match(gate, /    if: always\(\)/)
    if (name === 'release-linux.yml')
      assert.match(gate, /^    if: always\(\) && needs\.admit\.result != 'skipped'$/m)
    assert.ok(gate.includes(`needs: [${jobs.filter(j => j !== 'runtime-budget').join(', ')}]`), name)
    assert.match(gate, /actions: read/)
    assert.match(gate, /uses: .\/.github\/actions\/runtime-budget/)
  }
})


test('Linux folds preflight into the complete suite and overlaps its required app build; Mac preserves focused preflight', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/verification-platform.yml', import.meta.url), 'utf8')
  const block = workflow.split('      - name: Execute Rust changed-tree checks (not GUI acceptance)\n')[1].split('      - name: Actual Electron')[0]
  const script = block.split('        run: |\n')[1].split('\n').map(line => line.replace(/^          /, '')).join('\n').replaceAll('${{ needs.select.outputs.root }}', 'true')
  const root = mkdtempSync(join(tmpdir(), 'verification-scheduling-'))
  try {
    for (const name of ['cargo', 'python3']) writeFileSync(join(root, name), `#!/bin/sh\nprintf '${name}:%s\\n' "$*"\n`, { mode: 0o700 })
    for (const platform of ['Linux', 'macOS']) {
      const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], { encoding: 'utf8', env: { ...process.env, HOME: root, PATH: root + ':' + process.env.PATH, RUNNER_OS: platform } })
      assert.equal(result.status, 0, result.stderr)
      const cargo = result.stdout.split('\n').filter(line => line.startsWith('cargo:'))
      assert.equal(cargo.length, platform === 'Linux' ? 0 : 1, 'Linux must not serialize a cold test compilation before the complete verifier')
      if (platform === 'macOS') {
        assert.match(cargo[0], /the_command_line_row_/)
        assert.match(cargo[0], /competing_launches_cannot_remove_the_winning_control_socket/)
        assert.match(cargo[0], /hosted_security_darwin_acl_grants_fail_closed/)
      }
      const verifier = result.stdout.split('\n').find(line => line.startsWith('python3:'))!
      const emptyTree = spawnSync('git', ['hash-object', '-t', 'tree', '/dev/null'], { encoding: 'utf8' }).stdout.trim()
      assert.ok(verifier.includes('--base ' + emptyTree), 'empty-tree selection retains the complete root and Engine suites')
      assert.match(verifier, /--run --online --suite rust --parallel-contracts/)
      assert.match(verifier, /--overlap-root-build/)
      assert.doesNotMatch(verifier, /--focused|--skip/)
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('watched workflow starts have no standalone watchdog to spawn', () => {
  const folder = new URL('../../.github/workflows/', import.meta.url)
  for (const name of readdirSync(folder)) {
    const text = readFileSync(new URL(name, folder), 'utf8')
    assert.doesNotMatch(text, /Workflow runtime watchdog|--watch|runtime-watchdog-/)
    assert.doesNotMatch(text, /types: \[in_progress\]/)
  }
  const audit = readFileSync(new URL('../../.github/actions/runtime-budget/action.yml', import.meta.url), 'utf8')
  assert.match(audit, /workflow-budget.ts/)
  assert.doesNotMatch(audit, /--watch|actions: write|\/cancel/)
})

test('completion audit CLI fails an overrun without cancelling the underlying workflow', () => {
  const script = fileURLToPath(new URL('./workflow-budget.ts', import.meta.url))
  for (const [platform, seconds, expected] of [['linux', 599, 0], ['linux', 600, 0], ['linux', 601, 1], ['macos', 899, 0], ['macos', 900, 0], ['macos', 901, 1], ['invalid', 1, 1]] as const) {
    const bootstrap = `globalThis.fetch = async (_url, init) => {
      if (init?.method && init.method !== 'GET') throw new Error('Unexpected workflow mutation');
      return new Response(JSON.stringify({ total_count: 1, jobs: [${JSON.stringify(job(0, seconds))}] }));
    }; process.argv = [process.execPath, ${JSON.stringify(script)}]; await import(${JSON.stringify(new URL('./workflow-budget.ts', import.meta.url).href)});`
    const result = spawnSync(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', bootstrap], {
      encoding: 'utf8', env: { ...process.env, GITHUB_API_URL: 'https://api.github.com', GITHUB_REPOSITORY: 'synthetic/repo',
        GITHUB_RUN_ID: '42', GITHUB_RUN_ATTEMPT: '1', GH_TOKEN: 'synthetic', GITHUB_STEP_SUMMARY: '', WORKFLOW_BUDGET_PLATFORM: platform },
    })
    assert.equal(result.status, expected, result.stderr)
    if (expected) assert.match(result.stderr, platform === 'invalid' ? /Invalid workflow budget platform/ : /Optimize the tests/)
    assert.doesNotMatch(result.stderr, /Unexpected workflow mutation/)
  }
})

test('obsolete watch invocation fails before making any API request', () => {
  const result = spawnSync(process.execPath, ['--experimental-strip-types', fileURLToPath(new URL('./workflow-budget.ts', import.meta.url)), '--watch'], { encoding: 'utf8' })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /completion-time audit only/)
})

test('Mac alone has a fifteen-minute budget with strict boundaries', () => {
  assert.equal(workflowBudgetSeconds(), 600)
  assert.equal(workflowBudgetSeconds('macos'), 900)
  assert.throws(() => workflowBudgetSeconds('windows'), /Invalid/)
  assert.doesNotThrow(() => enforceBudget([job(0, 900)], 'macos'))
  assert.throws(() => enforceBudget([job(0, 901)], 'macos'), /900s/)
  assert.throws(() => enforceBudget([job(0, 601)], 'linux'), /600s/)
  const folder = new URL('../../.github/workflows/', import.meta.url)
  for (const name of ['verification-mac.yml', 'native-proof-mac.yml', 'release-mac-preview.yml']) {
    const text = readFileSync(new URL(name, folder), 'utf8')
    assert.match(text.split('  runtime-budget:\n')[1], /platform: macos/)
    assert.doesNotMatch(text, /^    timeout-minutes: 10$/m)
  }
  for (const name of ['verification.yml', 'release-linux.yml', 'web-delivery.yml'])
    assert.doesNotMatch(readFileSync(new URL(name, folder), 'utf8'), /platform: macos|timeout-minutes: 15/)
})

test('Electron to Rust Settings workflow builds frontend artifacts before the bridge test', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/verification-platform.yml', import.meta.url), 'utf8')
  const block = workflow.split('      - name: Actual Electron to Rust Settings integration (not native GUI proof)\n')[1].split('\n      - name: Install the Mac candidate')[0]
  const script = block.split('        run: |\n')[1].split('\n').map(line => line.replace(/^          /, '')).join('\n')
  const root = mkdtempSync(join(tmpdir(), 'electron-settings-workflow-'))
  try {
    const calls = join(root, 'calls')
    for (const binary of ['electron', 'Electron.app/Contents/MacOS/Electron']) {
      const file = join(root, 'capture/desktop/ui/frontend/node_modules/electron/dist', binary)
      mkdirSync(join(file, '..'), { recursive: true })
      writeFileSync(file, '#!/bin/sh\nexit 0\n', { mode: 0o700 })
    }
    writeFileSync(join(root, 'npm'), `#!/bin/sh
printf 'npm:%s\\n' "$*" >> "$CALLS"
case "$*" in *'run build'*) exit "$BUILD_STATUS";; esac
`, { mode: 0o700 })
    const sandbox = join(root, 'capture/desktop/ui/frontend/node_modules/electron/dist/chrome-sandbox')
    writeFileSync(sandbox, 'synthetic helper')
    writeFileSync(join(root, 'sudo'), `#!/bin/sh
printf 'sudo:%s\\n' "$*" >> "$CALLS"
exit "${'${SUDO_STATUS:-0}'}"
`, { mode: 0o700 })
    writeFileSync(join(root, 'stat'), '#!/bin/sh\nprintf "%s\\n" "$HELPER_STAT"\n', { mode: 0o700 })
    writeFileSync(join(root, 'xvfb-run'), '#!/bin/sh\nprintf "xvfb:%s\\n" "$*" >> "$CALLS"\nshift\nexec "$@"\n', { mode: 0o700 })
    for (const name of ['cargo', 'node']) writeFileSync(join(root, name), `#!/bin/sh
printf '${name}:%s\\n' "$*" >> "$CALLS"
`, { mode: 0o700 })
    for (const os of ['Linux', 'macOS']) for (const status of ['0', '7']) {
      writeFileSync(calls, '')
      const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], { cwd: root, encoding: 'utf8',
        env: { ...process.env, PATH: `${root}:${process.env.PATH}`, CALLS: calls, BUILD_STATUS: status, HOME: root, RUNNER_OS: os, GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted', HELPER_STAT: '0:0:4755' } })
      assert.equal(result.status, Number(status), result.stderr)
      const lines = readFileSync(calls, 'utf8').trim().split('\n')
      assert.equal(lines[0], 'npm:--prefix capture/desktop/ui/frontend ci --no-audit --no-fund')
      assert.equal(lines[1], 'npm:--prefix capture/desktop/ui/frontend run build')
      if (status === '0') {
        assert.match(lines[2], /^node:-e /)
        assert.deepEqual(lines.slice(3), os === 'Linux' ? [
          `sudo:chown root:root -- ${sandbox}`, `sudo:chmod 4755 -- ${sandbox}`,
          'xvfb:-a cargo test -j 3 --locked _electron_rust_roundtrip -- --ignored --nocapture',
          'cargo:test -j 3 --locked _electron_rust_roundtrip -- --ignored --nocapture',
        ] : ['cargo:test -j 3 --locked _electron_rust_roundtrip -- --ignored --nocapture'])
      } else assert.equal(lines.length, 2, 'build failure must stop before Electron or Cargo')
    }
    for (const scenario of ['not-hosted', 'wrong-mode', 'sudo-failure', 'symlink', 'missing']) {
      writeFileSync(calls, '')
      if (scenario === 'symlink' || scenario === 'missing') {
        rmSync(sandbox)
        if (scenario === 'symlink') symlinkSync(join(root, 'npm'), sandbox)
      }
      const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], { cwd: root, encoding: 'utf8',
        env: { ...process.env, PATH: `${root}:${process.env.PATH}`, CALLS: calls, BUILD_STATUS: '0', HOME: root,
          RUNNER_OS: 'Linux', GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: scenario === 'not-hosted' ? 'self-hosted' : 'github-hosted',
          HELPER_STAT: scenario === 'wrong-mode' ? '1000:1000:755' : '0:0:4755', SUDO_STATUS: scenario === 'sudo-failure' ? '9' : '0' } })
      assert.notEqual(result.status, 0, scenario)
      const recorded = readFileSync(calls, 'utf8')
      assert.doesNotMatch(recorded, /^cargo:|^xvfb:/m, scenario)
      if (['not-hosted', 'symlink', 'missing'].includes(scenario)) assert.doesNotMatch(recorded, /^sudo:/m, scenario)
      if (scenario === 'symlink' || scenario === 'missing') { rmSync(sandbox, { force: true }); writeFileSync(sandbox, 'synthetic helper') }
    }
  } finally { rmSync(root, { recursive: true, force: true }) }
})

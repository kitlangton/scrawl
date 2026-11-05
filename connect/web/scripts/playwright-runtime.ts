import { spawnSync, type SpawnSyncReturns } from 'node:child_process'

type Run = (command: string, args: string[]) => SpawnSyncReturns<Buffer>

const dependencyFailure = /Host system is missing dependencies|error while loading shared libraries|cannot open shared object file/i

function commandSucceeded(result: SpawnSyncReturns<Buffer>, command: string): void {
  if (result.stdout?.length) process.stdout.write(result.stdout)
  if (result.stderr?.length) process.stderr.write(result.stderr)
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} failed with exit ${result.status}`)
}

/** Install exact locked browsers, then prove both launch before accepting the runner environment. */
export function preparePlaywrightRuntime(run: Run = (command, args) => spawnSync(command, args, { encoding: 'buffer' })) {
  const cli = './node_modules/.bin/playwright-core'
  const launch = `import { chromium, firefox } from 'playwright-core'; for (const browser of [chromium, firefox]) { const instance = await browser.launch({headless:true,args:['--no-sandbox']}); await instance.close() }`
  commandSucceeded(run(cli, ['install', 'chromium', 'firefox']), 'Playwright browser install')
  const probe = () => run('node', ['--input-type=module', '-e', launch])
  const first = probe()
  if (first.status === 0 && !first.error) return
  const detail = `${first.stderr?.toString() ?? ''}\n${first.stdout?.toString() ?? ''}`
  if (!dependencyFailure.test(detail)) {
    throw new Error(`Playwright browser launch failed for a reason other than a missing system dependency: ${detail.trim() || first.error?.message || `exit ${first.status}`}`)
  }
  commandSucceeded(run(cli, ['install-deps', 'chromium', 'firefox']), 'Playwright system dependency install')
  const afterInstall = probe()
  commandSucceeded(afterInstall, 'Playwright browser launch after dependency install')
}

if (process.argv[1]?.endsWith('/playwright-runtime.ts')) {
  try { preparePlaywrightRuntime() }
  catch (error) {
    console.error(`Playwright runner readiness failed closed: ${String(error).replace(/[\r\n]/g, ' ')}`)
    process.exitCode = 1
  }
}

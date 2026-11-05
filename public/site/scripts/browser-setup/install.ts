import { execFileSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { installUbuntuDependencies, type UbuntuSetup } from '../../../../composition/delivery/runtime/ubuntu-dependencies.ts'

const site = fileURLToPath(new URL('../../', import.meta.url))
type Run = (command: string, args: string[], env: NodeJS.ProcessEnv) => void
interface Setup {
  environment?: NodeJS.ProcessEnv
  run?: Run
  browserPaths?: () => Promise<{ chromium: string; firefox: string }>
  osRelease?: string
  dependencies?: (setup: UbuntuSetup, installer: { command: string; args: string[] }) => void
}

/** Keep dependency discovery with locked Playwright; only change CI apt transport/deadlines. */
export async function installPublicBrowsers(setup: Setup = {}): Promise<string> {
  const environment = setup.environment ?? process.env
  const run: Run = setup.run ?? ((command, args, env) => {
    execFileSync(command, args, { cwd: site, env, stdio: 'inherit' })
  })
  const cli = join(site, 'node_modules/playwright-core/cli.js')
  const browsers = environment.CHROME_PATH ? ['firefox'] : ['chromium', 'firefox']
  if (environment.GITHUB_ACTIONS === 'true' && environment.RUNNER_OS === 'Linux') {
    const directory = mkdtempSync(join(tmpdir(), 'apparent-public-browser-apt-'))
    // Both browser runtimes are required, including when Chrome is supplied externally.
    const installDependencies = setup.dependencies ?? installUbuntuDependencies
    installDependencies({ directory,
      osRelease: setup.osRelease ?? readFileSync('/etc/os-release', 'utf8'), environment,
      root: process.getuid?.() === 0 }, {
      command: process.execPath, args: [cli, 'install-deps', 'chromium', 'firefox'],
    })
    run(process.execPath, [cli, 'install', ...browsers], environment)
  } else {
    run(process.execPath, [cli, 'install', '--with-deps', ...browsers], environment)
  }
  const paths = await (setup.browserPaths ?? (async () => {
    const { chromium, firefox } = await import('playwright-core')
    return { chromium: chromium.executablePath(), firefox: firefox.executablePath() }
  }))()
  const chrome = environment.CHROME_PATH ?? paths.chromium
  for (const path of [chrome, paths.firefox]) {
    if (!existsSync(path)) throw new Error(`Installed browser is missing: ${path}`)
  }
  return chrome
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await installPublicBrowsers()
}

import { execFileSync } from 'node:child_process'
import { chmodSync, mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const sources = `Types: deb
URIs: https://archive.ubuntu.com/ubuntu/
Suites: noble noble-updates noble-backports
Components: main universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: https://security.ubuntu.com/ubuntu/
Suites: noble-security
Components: main universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
`

type Run = (command: string, args: string[], options: { env: NodeJS.ProcessEnv; stdio: 'inherit'; timeout: number }) => unknown
export interface UbuntuSetup {
  directory: string
  osRelease: string
  environment: NodeJS.ProcessEnv
  root: boolean
  run?: Run
}

/** Disposable Ubuntu runner only; neither global apt sources nor trust settings are changed. */
export function installUbuntuDependencies(setup: UbuntuSetup, install: readonly string[] | { command: string; args: string[] }): void {
  if (setup.environment.GITHUB_ACTIONS !== 'true' || setup.environment.RUNNER_OS !== 'Linux'
    || setup.environment.ImageOS !== 'ubuntu24' || !/^ID=ubuntu$/m.test(setup.osRelease)
    || !/^VERSION_CODENAME=noble$/m.test(setup.osRelease)) throw new Error('Dependencies require the declared Ubuntu 24.04 runner')
  // These public repository URLs/metadata must be readable by apt's sandbox user.
  chmodSync(setup.directory, 0o755)
  const source = join(setup.directory, 'ubuntu.sources')
  const lists = join(setup.directory, 'lists')
  mkdirSync(lists, { mode: 0o755 })
  writeFileSync(source, sources, { flag: 'wx', mode: 0o644 })
  const apt = [
    '-o', `Dir::Etc::sourcelist=${source}`, '-o', 'Dir::Etc::sourceparts=-',
    '-o', `Dir::State::lists=${lists}`, '-o', 'Acquire::http::Timeout=20',
    '-o', 'Acquire::https::Timeout=20', '-o', 'Acquire::Retries=1',
    '-o', 'APT::Update::Error-Mode=any',
  ]
  const run = setup.run ?? execFileSync
  const execute = (seconds: number, operation: string[]) => {
    // timeout owns the process group even through sudo: a stalled apt child may
    // not survive the deadline and retain dpkg locks while the build continues.
    const args = ['--signal=TERM', '--kill-after=10s', `${seconds}s`, 'apt-get', ...apt, ...operation]
    run(setup.root ? 'timeout' : 'sudo', setup.root ? args : ['-n', 'env', 'DEBIAN_FRONTEND=noninteractive', 'timeout', ...args], {
      env: { ...setup.environment, DEBIAN_FRONTEND: 'noninteractive' }, stdio: 'inherit', timeout: (seconds + 15) * 1_000,
    })
  }
  if (Array.isArray(install)) {
    execute(60, ['update'])
    execute(180, ['install', '--no-install-recommends', '-y', ...install])
  } else {
    const installer = install as { command: string; args: string[] }
    // Playwright owns the package list. Its two apt calls inherit this isolated
    // configuration even when the installer crosses sudo's environment boundary.
    const config = join(setup.directory, 'apt.conf')
    writeFileSync(config, 'Dir::Etc::parts "-";\nDir::Etc::main "-";\n' + apt.filter((_, i) => i % 2 === 1).map(option => {
      const split = option.indexOf('=')
      return option.slice(0, split) + ' "' + option.slice(split + 1) + '";'
    }).join('\n') + '\n', { flag: 'wx', mode: 0o644 })
    const args = ['env', 'DEBIAN_FRONTEND=noninteractive', 'APT_CONFIG=' + config,
      'timeout', '--signal=TERM', '--kill-after=10s', '240s', installer.command, ...installer.args]
    run(setup.root ? args[0] : 'sudo', setup.root ? args.slice(1) : ['-n', ...args], {
      env: { ...setup.environment, DEBIAN_FRONTEND: 'noninteractive' }, stdio: 'inherit', timeout: 255_000,
    })
  }
}

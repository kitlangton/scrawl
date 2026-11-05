import { execFileSync } from 'node:child_process'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

export const mediaPackages = [
  'ffmpeg', 'pkg-config', 'libgstreamer1.0-dev', 'libgstreamer-plugins-base1.0-dev',
  'gstreamer1.0-plugins-base', 'gstreamer1.0-plugins-good', 'gstreamer1.0-libav',
] as const

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
interface Setup {
  directory: string
  osRelease: string
  environment: NodeJS.ProcessEnv
  root: boolean
  run?: Run
}

/** Disposable Ubuntu runner only; neither global apt sources nor trust settings are changed. */
export function installMediaDependencies(setup: Setup): void {
  if (setup.environment.GITHUB_ACTIONS !== 'true' || setup.environment.RUNNER_OS !== 'Linux'
    || setup.environment.ImageOS !== 'ubuntu24' || !/^ID=ubuntu$/m.test(setup.osRelease)
    || !/^VERSION_CODENAME=noble$/m.test(setup.osRelease)) throw new Error('Media dependencies require the declared Ubuntu 24.04 runner')
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
  execute(60, ['update'])
  execute(180, ['install', '--no-install-recommends', '-y', ...mediaPackages])
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  // Apt owns files beneath this unique temporary directory; the disposable
  // runner removes them with its other job state. Nothing here is an artifact.
  const directory = mkdtempSync(join(tmpdir(), 'apparent-media-apt-'))
  installMediaDependencies({ directory, osRelease: readFileSync('/etc/os-release', 'utf8'),
    environment: process.env, root: process.getuid?.() === 0 })
}

import { mkdtempSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { installUbuntuDependencies, type UbuntuSetup } from './ubuntu-dependencies.ts'

// Locked Playwright owns Electron's Chromium dependency list; the shared
// installer changes only apt transport, trust-preserving sources and deadlines.
export function installElectronDependencies(setup: UbuntuSetup): void {
  const cli = fileURLToPath(new URL('../../app/node_modules/playwright/cli.js', import.meta.url))
  installUbuntuDependencies(setup, { command: process.execPath, args: [cli, 'install-deps', 'chromium'] })
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  installElectronDependencies({ directory: mkdtempSync(join(tmpdir(), 'apparent-electron-apt-')),
    osRelease: readFileSync('/etc/os-release', 'utf8'), environment: process.env,
    root: process.getuid?.() === 0 })
}

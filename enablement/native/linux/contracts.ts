/** Launch-free checks shared by local development and Platform CI. */
import { appendFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../../../', import.meta.url))
const checks: [string, string[]][] = [
  ['python3', ['enablement/native/linux/test-gui-vm.py']],
  [process.execPath, ['--experimental-strip-types', '--experimental-vm-modules', '--test', 'enablement/native/linux/fixture-closure.test.ts']],
  ['python3', ['capture/desktop/scripts/linux/gui/test-capture-smoke.py']],
  ['python3', ['capture/desktop/scripts/linux/gui/test-narration-durability.py']],
]
const scope = 'Driver contracts only: no VM boot, desktop capture, microphone, ink, clipboard or installed-app acceptance.'

if (process.argv.length > 3 || (process.argv[2] && process.argv[2] !== '--list')) {
  console.error('Usage: node --experimental-strip-types enablement/native/linux/contracts.ts [--list]')
  process.exit(2)
}
console.log(scope)
if (process.argv[2] === '--list') {
  for (const [command, args] of checks) console.log([command, ...args].join(' '))
} else {
  if (process.platform !== 'linux') throw new Error('Linux driver contracts require Linux; native platform checks remain separate')
  for (const [command, args] of checks) {
    console.log(`Running ${args.at(-1)}`)
    const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' })
    if (result.error) throw result.error
    if (result.status !== 0) process.exit(result.status ?? 1)
  }
  console.log(`PASS: ${scope}`)
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY,
    `### Linux native driver contracts\n\n${scope}\n\nThe maintained runner, staging and scenario contract suites passed. Required native claims remain separately verified or UNRUN.\n`)
}

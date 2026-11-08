// Controlled synthetic API only. Never overwrite the production artifact with loopback config.
import { createServer } from 'node:net'
import { spawnSync } from 'node:child_process'

const socket = createServer()
await new Promise(r => socket.listen(0, '127.0.0.1', r))
const origin = `http://127.0.0.1:${socket.address().port}`
await new Promise(r => socket.close(r))
function run(command, args, env = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', env: { ...process.env, ...env } })
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed: ${result.error || result.status}`)
}
// Separate output keeps the production artifact free of loopback configuration.
run('npm', ['run', 'build'], { VITE_HOSTED_API_ORIGIN: origin, CONNECT_BUILD_DIR: 'dist-test' })
run('node', ['test/browser.mjs'], { VITE_HOSTED_API_ORIGIN: origin, HOSTED_TEST_API_ORIGIN: origin, CONNECT_ARTIFACT: 'dist-test' })
import { spawnSync } from 'node:child_process'
import { readFile, writeFile, rm } from 'node:fs/promises'
import { resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const runtime = resolve(root, '.output/runtime')
const build = spawnSync(resolve(root, 'node_modules/.bin/wrangler'), ['deploy', '--dry-run', '--outdir', runtime, '--config', 'wrangler.template.jsonc'], { cwd: root, stdio: 'inherit', env: { ...process.env, WRANGLER_SEND_METRICS: 'false' } })
if (build.error) throw build.error
if (build.status !== 0) throw new Error('Static Worker bundle failed')
await writeFile(resolve(root, '.output/worker.js'), await readFile(resolve(runtime, 'index.js')))
await rm(runtime, { recursive: true, force: true })
const template = JSON.parse(await readFile(resolve(root, 'wrangler.template.jsonc'), 'utf8'))
delete template.$schema
template.main = 'worker.js'
template.assets.directory = 'dist'
await writeFile(resolve(root, '.output/wrangler.json'), JSON.stringify(template, null, 2) + '\n')

import { spawnSync } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const service = resolve(root, '../../service')
const output = resolve(root, '.output')
const build = spawnSync(resolve(service, 'node_modules/.bin/esbuild'), [resolve(root, 'src/index.ts'), '--bundle', '--format=esm', '--platform=browser', '--conditions=workerd', '--external:node:*', `--outfile=${resolve(output, 'worker.js')}`], { cwd: root, stdio: 'inherit' })
if (build.error) throw build.error
if (build.status !== 0) throw new Error('Connect gateway Worker bundle failed')
const template = JSON.parse(await readFile(resolve(root, 'wrangler.template.jsonc'), 'utf8'))
delete template.$schema
template.main = 'worker.js'
template.assets.directory = 'dist'
await writeFile(resolve(root, '.output/wrangler.json'), JSON.stringify(template, null, 2) + '\n')

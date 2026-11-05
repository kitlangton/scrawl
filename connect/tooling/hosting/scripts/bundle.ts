import { spawnSync } from 'node:child_process'
import { readFile, writeFile, rm } from 'node:fs/promises'
import { resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const service = resolve(root, '../../service')
const install = spawnSync('npm', ['ci'], { cwd: service, stdio: 'inherit' })
if (install.error) throw install.error
if (install.status !== 0) throw new Error('Connect service dependency install failed')
const serviceBuild = spawnSync('npm', ['run', 'build'], { cwd: service, stdio: 'inherit' })
if (serviceBuild.error) throw serviceBuild.error
if (serviceBuild.status !== 0) throw new Error('Connect service build failed')
const output = resolve(root, '.output')
const build = spawnSync(resolve(service, 'node_modules/.bin/esbuild'), ['src/worker.ts', '--bundle', '--format=esm', '--platform=browser', '--conditions=workerd', '--external:node:*', `--outfile=${resolve(output, 'worker.js')}`], { cwd: service, stdio: 'inherit' })
if (build.error) throw build.error
if (build.status !== 0) throw new Error('Connect service Worker bundle failed')
const template = JSON.parse(await readFile(resolve(root, 'wrangler.template.jsonc'), 'utf8'))
delete template.$schema
template.main = 'worker.js'
template.assets.directory = 'dist'
await writeFile(resolve(root, '.output/wrangler.json'), JSON.stringify(template, null, 2) + '\n')

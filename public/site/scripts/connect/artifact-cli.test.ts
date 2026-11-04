import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, cpSync, rmSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { zipSync } from 'fflate'
import { consume, decodeArchive } from './artifact.ts'
import { seal, ROUTES } from '../../../../connect/delivery/artifacts.ts'

test('default committed producer CLI admits ancestor reuse and rejects changed candidate inputs', async () => {
  const root = mkdtempSync(join(tmpdir(), 'public-cli-consumer-'))
  const artifactRoot = join(root, 'artifact'), out = join(root, 'consumed')
  const put = (path: string, value = 'fixture') => { mkdirSync(join(root, path, '..'), { recursive: true }); writeFileSync(join(root, path), value) }
  const git = (...args: string[]) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
  const commit = () => { git('add', '.'); git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-m', 'fixture') }
  try {
    for (const path of ['connect/web/package.json', 'shared/ui/token.css', 'shared/analytics/config.ts', '.github/workflows/connect-delivery.yml', '.github/actions/runtime-budget/action.yml', 'enablement/verification/workflow-budget.ts','enablement/release_gate.py','enablement/verification/web/inputs.ts','enablement/verification/web/baseline.ts']) put(path)
    put('connect/delivery/artifacts.ts', readFileSync(new URL('../../../../connect/delivery/artifacts.ts', import.meta.url), 'utf8'))
    put('connect/web/scripts/assets.mjs', readFileSync(new URL('../../../../connect/web/scripts/assets.mjs', import.meta.url), 'utf8'))
    for (const path of ['hosted/source-pond-v4.png', 'hosted/agent-pond-v4.png', 'hosted/agent-reading-v4.png', 'fonts/HostGrotesk-Variable.ttf', 'brand/apparent-wordmark.svg']) put(`public/site/public/assets/${path}`)
    for (const route of ROUTES) put(`artifact/dist/${route}/index.html`, '<html>production fixture</html>')
    git('init', '-b', 'main'); commit()
    const sourceSha = git('rev-parse', 'HEAD')
    await seal(root, artifactRoot)
    // Fixture transport represents the pinned CI runtime; no real run is claimed.
    const manifest = JSON.parse(readFileSync(join(artifactRoot, 'manifest.json'), 'utf8'))
    manifest.build.node = '22.23.0'; manifest.build.bun = '1.3.10'
    writeFileSync(join(artifactRoot, 'manifest.json'), JSON.stringify(manifest))
    const files: Record<string, Uint8Array> = {}
    function walk(directory: string, prefix = '') { for (const entry of readdirSync(directory, { withFileTypes: true })) { const path = prefix + entry.name; if (entry.isDirectory()) walk(join(directory, entry.name), path + '/'); else files[path] = readFileSync(join(directory, entry.name)) } }
    walk(artifactRoot)
    const bytes = Buffer.from(zipSync(files)), digest = 'sha256:' + createHash('sha256').update(bytes).digest('hex')
    const run = { id: 42, status: 'completed', conclusion: 'success', event: 'push', path: '.github/workflows/connect-delivery.yml', head_branch: 'main', head_sha: sourceSha, repository: { full_name: 'tryapparent/apparent' }, head_repository: { full_name: 'tryapparent/apparent' } }
    const api = async (path: string) => path.includes('/runs?') ? { workflow_runs: [run] } : { artifacts: [{ id: 1, name: `connect-browser-${sourceSha}`, expired: false, digest, workflow_run: { id: 42, head_sha: sourceSha } }] }
    put('public/site/src/home.tsx', 'website-only change'); commit()
    assert.equal((await consume(root, out, api, async () => bytes)).sourceSha, sourceSha)
    put('shared/ui/token.css', 'changed shared contract'); commit()
    await assert.rejects(consume(root, out, api, async () => bytes), /No successful/)
    put('shared/ui/token.css'); put('connect/web/new.ts', 'added product input'); commit()
    await assert.rejects(consume(root, out, api, async () => bytes), /No successful/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('decoder rejects duplicate ZIP entries and per-file/total expanded size limits', () => {
  const duplicate = Buffer.from(zipSync({ 'manifest.json': new Uint8Array(), 'dist/a': new Uint8Array(), 'dist/b': new Uint8Array() }))
  for (let offset = 0; (offset = duplicate.indexOf('dist/b', offset)) !== -1; offset += 6) duplicate.write('dist/a', offset)
  assert.throws(() => decodeArchive(duplicate), /Duplicate/)
  function oversized(names: string[], size: number) {
    const archive = Buffer.from(zipSync(Object.fromEntries(names.map(name => [name, new Uint8Array()]))))
    for (let index = 0; index < archive.length - 28; index++) if (archive.readUInt32LE(index) === 0x02014b50) archive.writeUInt32LE(size, index + 24)
    return archive
  }
  assert.throws(() => decodeArchive(oversized(['manifest.json'], 21 * 1024 * 1024)), /size limit/)
  assert.throws(() => decodeArchive(oversized(Array.from({ length: 6 }, (_, i) => `dist/d${i}/`), 20 * 1024 * 1024)), /size limit/)
})

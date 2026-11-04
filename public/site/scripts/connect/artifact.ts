/** Public consumes verified Connect bytes; it never installs or builds Connect. */
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { join, resolve, dirname } from 'node:path'
import { pathToFileURL } from 'node:url'
import { unzipSync } from 'fflate'
import { safePath } from './compose-connect.mjs'

const repository = 'tryapparent/apparent'
export const producerPath = '.github/workflows/connect-delivery.yml'
export function trustedRun(run: any, ancestor: (sha: string) => boolean) {
  return run.status === 'completed' && run.conclusion === 'success' &&
    run.head_branch === 'main' && run.head_repository?.full_name === repository &&
    run.repository?.full_name === repository && run.path === producerPath &&
    ['push', 'workflow_dispatch'].includes(run.event) &&
    /^[a-f0-9]{40}$/.test(run.head_sha) && ancestor(run.head_sha)
}
export function trustedArtifact(artifact: any, run: any) {
  if (artifact.expired || artifact.name !== `connect-browser-${run.head_sha}` ||
      !Number.isSafeInteger(artifact.id) || artifact.id < 1 ||
      !/^sha256:[a-f0-9]{64}$/.test(artifact.digest) ||
      artifact.workflow_run?.id !== run.id || artifact.workflow_run?.head_sha !== run.head_sha)
    throw Error('Invalid Connect artifact provenance')
  return artifact.digest.slice(7)
}
export function verifyArchive(bytes: Buffer, digest: string) {
  if (bytes.length > 100 * 1024 * 1024 || createHash('sha256').update(bytes).digest('hex') !== digest)
    throw Error('Connect archive digest mismatch or size limit')
}
export function archivePaths(listing: string) {
  const paths = listing.trimEnd().split('\n').filter(path => !path.endsWith('/'))
  if (!paths.length || new Set(paths).size !== paths.length) throw Error('Empty or duplicate archive paths')
  for (const path of paths) {
    safePath(path)
    if (path !== 'manifest.json' && !path.startsWith('dist/')) throw Error('Unexpected Connect archive path')
  }
  if (!paths.includes('manifest.json')) throw Error('Missing Connect manifest')
  return paths
}
export function compatibleBuild(build: any, env: NodeJS.ProcessEnv = process.env) {
  const expected = {
    VITE_HOSTED_API_ORIGIN: 'https://hosted.tryapparent.com',
    PUBLIC_POSTHOG_ENABLED: env.PUBLIC_POSTHOG_ENABLED ?? 'true',
    PUBLIC_POSTHOG_TOKEN: env.PUBLIC_POSTHOG_TOKEN ?? '',
    PUBLIC_POSTHOG_HOST: 'https://eu.i.posthog.com',
    PUBLIC_POSTHOG_ENVIRONMENT: 'production', NODE_ENV: 'production',
  }
  return build?.node === '22.23.0' && build?.bun === '1.3.10' &&
    Object.keys(build.environment ?? {}).length === Object.keys(expected).length &&
    Object.entries(expected).every(([key, value]) => build.environment[key] === value)
}
export function decodeArchive(bytes: Buffer) {
  let total = 0
  const seen = new Set<string>()
  const files = unzipSync(bytes, { filter(file) {
    if (seen.has(file.name)) throw Error('Duplicate archive path')
    seen.add(file.name)
    safePath(file.name.replace(/\/$/, ''))
    if (file.name !== 'manifest.json' && !file.name.startsWith('dist/')) throw Error('Unexpected archive path')
    total += file.originalSize
    if (file.originalSize > 20 * 1024 * 1024 || total > 100 * 1024 * 1024) throw Error('Expanded artifact size limit')
    return !file.name.endsWith('/')
  } })
  archivePaths(Object.keys(files).join('\n'))
  return files
}
export async function consume(root: string, out: string, api: (path: string) => Promise<any>, download: (id: number) => Promise<Buffer>, validate = (directory: string) => {
  execFileSync('node', ['--experimental-strip-types', 'connect/delivery/artifacts.ts', 'validate', directory, 'connect-browser', root], { cwd: root, stdio: 'pipe' })
}) {
  const ancestor = (sha: string) => {
    try { execFileSync('git', ['merge-base', '--is-ancestor', sha, 'HEAD'], { cwd: root }); return true } catch { return false }
  }
  // A bounded search fails closed when no retained compatible producer exists.
  const response = await api('/actions/workflows/connect-delivery.yml/runs?branch=main&status=success&per_page=100')
  for (const run of response.workflow_runs ?? []) {
    if (!trustedRun(run, ancestor)) continue
    const artifacts = await api(`/actions/runs/${run.id}/artifacts?per_page=100`)
    const artifact = artifacts.artifacts?.find((item: any) => item.name === `connect-browser-${run.head_sha}` && !item.expired)
    if (!artifact) continue
    const digest = trustedArtifact(artifact, run)
    const bytes = await download(artifact.id)
    verifyArchive(bytes, digest)
    const files = decodeArchive(bytes)
    {
      rmSync(out, { recursive: true, force: true }); mkdirSync(out, { recursive: true })
      for (const [path, contents] of Object.entries(files)) {
        const destination = join(out, path)
        mkdirSync(dirname(destination), { recursive: true })
        // Stream entries as regular bytes; archive links are never extracted.
        writeFileSync(destination, contents)
      }
      const manifest = JSON.parse(readFileSync(join(out, 'manifest.json'), 'utf8'))
      if (!compatibleBuild(manifest.build)) continue
      if (manifest.sourceSha !== run.head_sha) throw Error('Producer source mismatch')
      try {
        validate(out)
      } catch { continue } // Verified ancestor may have different inputs; try an older retained artifact.
      return { sourceSha: run.head_sha, runId: run.id, artifactId: artifact.id, digest: artifact.digest }
    }
  }
  rmSync(out, { recursive: true, force: true })
  throw Error('No successful trusted main Connect artifact matches candidate inputs and build configuration')
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const root = resolve(import.meta.dirname, '../../../..'), out = resolve(process.argv[2] ?? '.build/connect-browser')
  const token = process.env.GH_TOKEN
  if (!token) throw Error('Missing Actions read token')
  const request = async (path: string) => {
    const response = await fetch(`https://api.github.com/repos/${repository}${path}`, { headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' } })
    if (!response.ok) throw Error(`Actions API ${response.status}`)
    return response
  }
  const receipt = await consume(root, out, async path => (await request(path)).json(), async id => Buffer.from(await (await request(`/actions/artifacts/${id}/zip`)).arrayBuffer()))
  writeFileSync(`${out}-receipt.json`, JSON.stringify(receipt, null, 2) + '\n')
  console.log(JSON.stringify(receipt))
}

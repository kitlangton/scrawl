/** Consume only the verified Public web artifact from the trusted value-stream workflow. */
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { gunzipSync } from 'node:zlib'
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path'
import { pathToFileURL } from 'node:url'
import { unzipSync } from 'fflate'

const repository = 'tryapparent/apparent'
export const workflowPath = '.github/workflows/value-stream-delivery.yml'
export const ownerId = 'public'
export const target = 'web'
const maxZipBytes = 512 * 1024 * 1024
const maxTarBytes = 1024 * 1024 * 1024
const maxFileBytes = 256 * 1024 * 1024

type Run = { id: number; run_attempt: number; status: string; conclusion: string | null; event: string; path: string; head_branch: string; head_sha: string; repository?: { full_name?: string }; head_repository?: { full_name?: string } }
type Deps = {
  sourceDigest?: (root: string, owner: any, revision?: string) => string
  environmentDigest?: (owner: any, env?: NodeJS.ProcessEnv) => string
  ancestor?: (sha: string) => boolean
  owner?: any
}

const sha256 = (bytes: Uint8Array | string) => createHash('sha256').update(bytes).digest('hex')

export function trustedRun(run: Run, ancestor: (sha: string) => boolean): boolean {
  return run.status === 'completed' && ['push', 'workflow_dispatch'].includes(run.event) &&
    run.head_branch === 'main' && run.head_repository?.full_name === repository &&
    run.repository?.full_name === repository && run.path === workflowPath &&
    /^[a-f0-9]{40}$/.test(run.head_sha) && Number.isSafeInteger(run.id) && run.id > 0 &&
    Number.isSafeInteger(run.run_attempt) && run.run_attempt > 0 && ancestor(run.head_sha)
}

export function validateJobs(jobs: any[], run: Run): void {
  const required = ['stream (public, web)', 'Delivery contracts', 'Workflow runtime budget']
  for (const name of required) {
    const matches = jobs.filter(job => job.name === name)
    if (matches.length !== 1 || matches[0].status !== 'completed' || matches[0].conclusion !== 'success' ||
        matches[0].run_id !== run.id || matches[0].run_attempt !== run.run_attempt) {
      throw new Error(`Required value-stream job did not succeed exactly once: ${name}`)
    }
  }
}

export function validateArtifact(artifact: any, run: Run, inputDigest: string): string {
  if (artifact.expired || artifact.name !== `stream-public-web-${inputDigest}` ||
      !Number.isSafeInteger(artifact.id) || artifact.id < 1 ||
      !/^sha256:[a-f0-9]{64}$/.test(artifact.digest) ||
      artifact.workflow_run?.id !== run.id || artifact.workflow_run?.head_sha !== run.head_sha) {
    throw new Error('Invalid Public artifact provenance')
  }
  return artifact.digest.slice('sha256:'.length)
}

function rejectZipLinks(bytes: Buffer) {
  let eocd = -1
  for (let offset = bytes.length - 22; offset >= Math.max(0, bytes.length - 65557); offset--) {
    if (bytes.readUInt32LE(offset) !== 0x06054b50) continue
    const commentLength = bytes.readUInt16LE(offset + 20)
    if (offset + 22 + commentLength !== bytes.length) continue
    eocd = offset
    break
  }
  if (eocd < 0 || bytes.readUInt16LE(eocd + 4) !== 0 || bytes.readUInt16LE(eocd + 6) !== 0 ||
      bytes.readUInt16LE(eocd + 8) !== bytes.readUInt16LE(eocd + 10)) throw new Error('Invalid Public artifact ZIP directory')
  const count = bytes.readUInt16LE(eocd + 10), directorySize = bytes.readUInt32LE(eocd + 12), directoryOffset = bytes.readUInt32LE(eocd + 16)
  if (count === 0xffff || directorySize === 0xffffffff || directoryOffset === 0xffffffff || directoryOffset + directorySize !== eocd) throw new Error('Unsupported Public artifact ZIP directory')
  let cursor = directoryOffset
  for (let index = 0; index < count; index++) {
    if (cursor + 46 > eocd || bytes.readUInt32LE(cursor) !== 0x02014b50) throw new Error('Invalid Public artifact ZIP entry')
    const nameLength = bytes.readUInt16LE(cursor + 28), extraLength = bytes.readUInt16LE(cursor + 30), commentLength = bytes.readUInt16LE(cursor + 32)
    const attributes = bytes.readUInt32LE(cursor + 38)
    const kind = (attributes >>> 16) & 0o170000
    if (kind !== 0 && kind !== 0o100000) throw new Error('Public ZIP entries must be regular files')
    cursor += 46 + nameLength + extraLength + commentLength
  }
  if (cursor !== eocd) throw new Error('Truncated Public artifact ZIP directory')
}
function decodeZip(bytes: Buffer, expectedZipDigest: string) {
  if (bytes.length > maxZipBytes || sha256(bytes) !== expectedZipDigest) throw new Error('Public artifact ZIP digest mismatch or size limit')
  rejectZipLinks(bytes)
  const seen = new Set<string>()
  const files = unzipSync(bytes, { filter(file: any) {
    if (seen.has(file.name)) throw new Error('Duplicate Public artifact ZIP path')
    seen.add(file.name)
    if (file.name !== 'component.tar.gz' && file.name !== 'receipt.json') throw new Error('Unexpected Public artifact ZIP path')
    if (file.originalSize > maxZipBytes) throw new Error('Public artifact ZIP expanded size limit')
    return true
  } })
  if (seen.size !== 2 || !seen.has('component.tar.gz') || !seen.has('receipt.json')) throw new Error('Public artifact ZIP must contain exactly receipt.json and component.tar.gz')
  return files as Record<string, Uint8Array>
}

type TarEntry = { path: string; kind: 'file' | 'directory'; mode: number; data?: Buffer }
function tarString(header: Buffer, start: number, length: number): string {
  const field = header.subarray(start, start + length)
  const nul = field.indexOf(0)
  return new TextDecoder('utf-8', { fatal: true }).decode(nul < 0 ? field : field.subarray(0, nul))
}
function tarOctal(header: Buffer, start: number, length: number, label: string): number {
  const raw = tarString(header, start, length).trim()
  if (!raw) return 0
  if (!/^[0-7]+$/.test(raw)) throw new Error(`Invalid tar ${label}`)
  const value = Number.parseInt(raw, 8)
  if (!Number.isSafeInteger(value)) throw new Error(`Invalid tar ${label}`)
  return value
}
function safeTarPath(path: string): string {
  if (!path || path.startsWith('/') || path.includes('\\') || /^[A-Za-z]:/.test(path) || isAbsolute(path)) throw new Error('Unsafe absolute Public tar path')
  const normalized = path.endsWith('/') ? path.slice(0, -1) : path
  if (!normalized || normalized.split('/').some(part => !part || part === '.' || part === '..')) throw new Error('Unsafe Public tar path')
  return normalized
}

/** Strictly parse ordinary USTAR files/directories; links, extensions and special files are rejected. */
export function decodePayloadTar(gzip: Uint8Array): TarEntry[] {
  const archive = gunzipSync(gzip, { maxOutputLength: maxTarBytes })
  const entries: TarEntry[] = [], seen = new Set<string>()
  let offset = 0, end = false, total = 0
  while (offset + 512 <= archive.length) {
    const header = archive.subarray(offset, offset + 512)
    if (header.every(byte => byte === 0)) {
      if (offset + 1024 > archive.length || !archive.subarray(offset + 512, offset + 1024).every(byte => byte === 0)) throw new Error('Invalid tar end marker')
      end = true
      if (!archive.subarray(offset + 1024).every(byte => byte === 0)) throw new Error('Unexpected data after tar end marker')
      break
    }
    const expectedChecksum = tarOctal(header, 148, 8, 'checksum')
    let checksum = 0
    for (let index = 0; index < 512; index++) checksum += index >= 148 && index < 156 ? 32 : header[index]
    if (checksum !== expectedChecksum) throw new Error('Public tar header checksum mismatch')
    const name = tarString(header, 0, 100), prefix = tarString(header, 345, 155)
    const path = safeTarPath(prefix ? `${prefix}/${name}` : name)
    const type = header[156] === 0 ? '0' : String.fromCharCode(header[156])
    const isDirectory = type === '5'
    if (type !== '0' && !isDirectory) throw new Error('Public tar may contain only regular files and directories')
    const normalizedPath = isDirectory ? path.replace(/\/$/, '') : path
    if (seen.has(normalizedPath)) throw new Error('Duplicate Public tar path')
    seen.add(normalizedPath)
    if (normalizedPath !== 'stream-payload' && normalizedPath !== 'stream-payload/payload' && !normalizedPath.startsWith('stream-payload/payload/')) throw new Error('Unexpected Public tar payload prefix')
    if (!isDirectory && !normalizedPath.startsWith('stream-payload/payload/')) throw new Error('Public tar file is outside payload')
    const size = isDirectory ? 0 : tarOctal(header, 124, 12, 'size')
    if (size > maxFileBytes || (total += size) > maxTarBytes) throw new Error('Public tar expanded size limit')
    const bodyStart = offset + 512, bodyEnd = bodyStart + size, next = bodyStart + Math.ceil(size / 512) * 512
    if (bodyEnd > archive.length || next > archive.length) throw new Error('Truncated Public tar entry')
    const mode = tarOctal(header, 100, 8, 'mode') & 0o777
    entries.push({ path: normalizedPath, kind: isDirectory ? 'directory' : 'file', mode, ...(isDirectory ? {} : { data: archive.subarray(bodyStart, bodyEnd) }) })
    offset = next
  }
  if (!end) throw new Error('Public tar has no complete end marker')
  if (!entries.some(entry => entry.kind === 'file')) throw new Error('Public tar payload is empty')
  return entries
}

function defaultCore() {
  // Loaded only when consuming so isolated contract tests can inject the digest functions.
  return import('../../../../composition/delivery/core.ts')
}
function gitAncestor(root: string, sha: string): boolean {
  try { execFileSync('git', ['merge-base', '--is-ancestor', sha, 'HEAD'], { cwd: root, stdio: 'ignore' }); return true }
  catch { return false }
}
function checkedOwner(core: any, root: string) {
  const result = core.catalog(root)
  const publicBlocked = (result.blocked ?? []).filter((message: string) => message.startsWith('public:'))
  if (publicBlocked.length) throw new Error(`Public value-stream registration is blocked: ${publicBlocked.join('; ')}`)
  const owner = result.owners.find((candidate: any) => candidate.id === ownerId)
  if (!owner || owner.kind !== 'directory' || owner.package !== 'public/site' || owner.artifactPath !== 'dist-apparent' || owner.stage !== 'website' || !owner.targets.includes(target)) throw new Error('Public web owner is not registered with the expected artifact contract')
  return owner
}

export async function consume(root: string, out: string, api: (path: string) => Promise<any>, download: (id: number) => Promise<Buffer>, deps: Deps = {}) {
  root = resolve(root); out = resolve(out)
  const receiptOut = `${out}-receipt.json`
  const ownerCore: any = await defaultCore()
  const owner = deps.owner ?? checkedOwner(ownerCore, root)
  const currentSource = (deps.sourceDigest ?? ownerCore.sourceDigest)(root, owner, 'HEAD')
  const currentEnvironment = (deps.environmentDigest ?? ownerCore.environmentDigest)(owner, process.env)
  if (!/^[a-f0-9]{64}$/.test(currentSource) || !/^[a-f0-9]{64}$/.test(currentEnvironment)) throw new Error('Invalid current Public input identity')
  const ancestor = deps.ancestor ?? ((sha: string) => gitAncestor(root, sha))
  const response = await api('/actions/workflows/value-stream-delivery.yml/runs?branch=main&status=completed&per_page=100')
  const runs: Run[] = response.workflow_runs ?? []
  for (const run of runs) {
    if (!trustedRun(run, ancestor)) continue
    if ((deps.sourceDigest ?? ownerCore.sourceDigest)(root, owner, run.head_sha) !== currentSource) continue
    const jobsResponse = await api(`/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`)
    if (!Array.isArray(jobsResponse.jobs) || jobsResponse.total_count !== jobsResponse.jobs.length) continue
    try { validateJobs(jobsResponse.jobs, run) } catch { continue }
    const artifactsResponse = await api(`/actions/runs/${run.id}/artifacts?per_page=100`)
    if (!Array.isArray(artifactsResponse.artifacts) || artifactsResponse.total_count !== artifactsResponse.artifacts.length) continue
    const expectedPrefix = 'stream-public-web-'
    const matching = (artifactsResponse.artifacts ?? []).filter((artifact: any) => typeof artifact.name === 'string' && artifact.name.startsWith(expectedPrefix) && !artifact.expired)
    for (const artifact of matching) {
      const inputDigest = artifact.name.slice(expectedPrefix.length)
      if (!/^[a-f0-9]{64}$/.test(inputDigest)) continue
      let expectedZipDigest: string
      try { expectedZipDigest = validateArtifact(artifact, run, inputDigest) } catch { continue }
      const bytes = await download(artifact.id)
      const archive = decodeZip(bytes, expectedZipDigest)
      let receipt: any
      try { receipt = JSON.parse(Buffer.from(archive['receipt.json']).toString('utf8')) }
      catch { throw new Error('Invalid Public stream receipt JSON') }
      if (receipt.schema !== 1 || receipt.id !== owner.id || receipt.target !== target || receipt.stage !== owner.stage ||
          receipt.inputDigest !== inputDigest || receipt.sourceSha !== run.head_sha ||
          receipt.runId !== String(run.id) || receipt.runAttempt !== String(run.run_attempt) ||
          !/^[a-f0-9]{64}$/.test(receipt.sha256) || !Number.isSafeInteger(receipt.size) || receipt.size <= 0 ||
          receipt.sourceDigest !== currentSource || receipt.environmentDigest !== currentEnvironment) continue
      const tarBytes = Buffer.from(archive['component.tar.gz'])
      if (tarBytes.length !== receipt.size || sha256(tarBytes) !== receipt.sha256) throw new Error('Public component tar digest or size mismatch')
      const entries = decodePayloadTar(tarBytes)
      if (exists(out) || exists(receiptOut)) throw new Error('Public artifact output and provenance receipt must not already exist')
      const parent = dirname(out)
      mkdirSync(parent, { recursive: true })
      const stage = mkdtempSync(join(parent, `.${out.split(sep).pop()}.stage-`))
      const receiptTemp = join(stage, 'receipt.json')
      let committed = false
      try {
        const payloadRoot = join(stage, 'payload')
        mkdirSync(payloadRoot)
        for (const entry of entries) {
          if (entry.path === 'stream-payload' || entry.path === 'stream-payload/payload') continue
          const rel = entry.path.slice('stream-payload/payload/'.length)
          if (!rel || rel.split('/').some(part => !part || part === '.' || part === '..')) throw new Error('Unsafe Public payload relative path')
          const destination = resolve(payloadRoot, rel)
          const containment = relative(payloadRoot, destination)
          if (!containment || containment === '..' || containment.startsWith(`..${sep}`) || isAbsolute(containment)) throw new Error('Public payload path escapes extraction root')
          if (entry.kind === 'directory') mkdirSync(destination, { recursive: true })
          else {
            mkdirSync(dirname(destination), { recursive: true })
            writeFileSync(destination, entry.data!, { flag: 'wx', mode: entry.mode & 0o111 ? 0o755 : 0o644 })
          }
        }
        const finalReceipt = { ...receipt, artifactId: artifact.id, artifactDigest: artifact.digest, workflowRunId: run.id, sourceSha: run.head_sha }
        writeFileSync(receiptTemp, JSON.stringify(finalReceipt, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
        renameSync(payloadRoot, out)
        try { renameSync(receiptTemp, receiptOut); committed = true }
        catch (error) { rmSync(out, { recursive: true, force: true }); throw error }
        return { sourceSha: run.head_sha, runId: run.id, runAttempt: run.run_attempt, artifactId: artifact.id, artifactDigest: artifact.digest, receipt: receiptOut }
      } finally {
        rmSync(stage, { recursive: true, force: true })
        if (!committed) rmSync(receiptTemp, { force: true })
      }
    }
  }
  throw new Error('No successful trusted main Public artifact matches current source and production configuration')
}

/** Consume the component directory downloaded from the current workflow run. */
export async function consumeCurrent(root: string, componentDir: string, out: string, deps: Deps = {}, env: NodeJS.ProcessEnv = process.env) {
  root = resolve(root); componentDir = resolve(componentDir); out = resolve(out)
  const ownerCore: any = await defaultCore()
  const owner = deps.owner ?? checkedOwner(ownerCore, root)
  const sourceSha = env.GITHUB_SHA ?? ''
  const runId = env.GITHUB_RUN_ID ?? '', runAttempt = env.GITHUB_RUN_ATTEMPT ?? ''
  const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  if (!/^[a-f0-9]{40}$/.test(sourceSha) || head !== sourceSha || !/^[1-9]\d*$/.test(runId) || !/^[1-9]\d*$/.test(runAttempt) ||
      env.GITHUB_REPOSITORY !== repository || env.GITHUB_REF !== 'refs/heads/main' ||
      env.GITHUB_WORKFLOW_REF !== `${repository}/${workflowPath}@refs/heads/main` ||
      owner.id !== ownerId || owner.kind !== 'directory' || owner.package !== 'public/site' || owner.artifactPath !== 'dist-apparent' || owner.stage !== 'website' || !owner.targets.includes(target)) {
    throw new Error('Current Public workflow identity or owner registration is invalid')
  }
  const source = (deps.sourceDigest ?? ownerCore.sourceDigest)(root, owner, sourceSha)
  const environment = (deps.environmentDigest ?? ownerCore.environmentDigest)(owner, env)
  const input = env.PUBLIC_COMPONENT_INPUT_DIGEST ?? ''
  if (![source, environment, input].every((value: string) => /^[a-f0-9]{64}$/.test(value))) throw new Error('Invalid current Public input identity')
  const componentStat = lstatSync(componentDir)
  if (!componentStat.isDirectory() || componentStat.isSymbolicLink()) throw new Error('Current Public artifact path must be a real directory')
  const receiptPath = join(componentDir, 'receipt.json'), tarPath = join(componentDir, 'component.tar.gz')
  if (readdirSync(componentDir).sort().join(',') !== 'component.tar.gz,receipt.json') throw new Error('Unexpected current Public artifact directory contents')
  for (const path of [receiptPath, tarPath]) {
    const stat = lstatSync(path)
    if (!stat.isFile() || stat.isSymbolicLink()) throw new Error('Current Public artifact entries must be regular files')
    if (stat.size > (path === receiptPath ? 1024 * 1024 : maxZipBytes)) throw new Error('Current Public artifact size limit')
  }
  const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'))
  const bytes = readFileSync(tarPath)
  if (receipt.schema !== 1 || receipt.id !== owner.id || receipt.target !== target || receipt.stage !== owner.stage ||
      receipt.sourceSha !== sourceSha || receipt.runId !== runId || receipt.runAttempt !== runAttempt ||
      receipt.sourceDigest !== source || receipt.environmentDigest !== environment || receipt.inputDigest !== input ||
      !/^[a-f0-9]{64}$/.test(receipt.sha256) || !Number.isSafeInteger(receipt.size) || receipt.size <= 0 ||
      bytes.length !== receipt.size || sha256(bytes) !== receipt.sha256) throw new Error('Current Public receipt provenance or payload digest mismatch')
  const entries = decodePayloadTar(bytes)
  const outReceipt = `${out}-receipt.json`
  if (exists(out) || exists(outReceipt)) throw new Error('Public artifact output and provenance receipt must not already exist')
  const parent = dirname(out); mkdirSync(parent, { recursive: true })
  const stage = mkdtempSync(join(parent, `.${out.split(sep).pop()}.stage-`)), receiptTemp = join(stage, 'receipt.json')
  let committed = false
  try {
    const payloadRoot = join(stage, 'payload'); mkdirSync(payloadRoot)
    for (const entry of entries) {
      if (entry.path === 'stream-payload' || entry.path === 'stream-payload/payload') continue
      const rel = entry.path.slice('stream-payload/payload/'.length)
      if (!rel || rel.split('/').some(part => !part || part === '.' || part === '..')) throw new Error('Unsafe Public payload relative path')
      const destination = resolve(payloadRoot, rel), containment = relative(payloadRoot, destination)
      if (!containment || containment === '..' || containment.startsWith(`..${sep}`) || isAbsolute(containment)) throw new Error('Public payload path escapes extraction root')
      if (entry.kind === 'directory') mkdirSync(destination, { recursive: true })
      else { mkdirSync(dirname(destination), { recursive: true }); writeFileSync(destination, entry.data!, { flag: 'wx', mode: entry.mode & 0o111 ? 0o755 : 0o644 }) }
    }
    writeFileSync(receiptTemp, JSON.stringify({ ...receipt, sourceSha }, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
    renameSync(payloadRoot, out)
    try { renameSync(receiptTemp, outReceipt); committed = true }
    catch (error) { rmSync(out, { recursive: true, force: true }); throw error }
    return { sourceSha, runId: Number(runId), runAttempt: Number(runAttempt), receipt: outReceipt }
  } finally { rmSync(stage, { recursive: true, force: true }); if (!committed) rmSync(receiptTemp, { force: true }) }
}
function exists(path: string) { try { lstatSync(path); return true } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false; throw error } }

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const root = resolve(import.meta.dirname, '../../../..')
  if (process.argv[2] === '--current') {
    const componentDir = resolve(process.argv[3] ?? '.build/public-component'), out = resolve(process.argv[4] ?? '.build/public-artifact')
    console.log(JSON.stringify(await consumeCurrent(root, componentDir, out)))
  } else {
  const out = resolve(process.argv[2] ?? '.build/public-artifact')
  const token = process.env.GH_TOKEN
  if (!token) throw new Error('Missing Actions read token')
  const request = async (path: string) => {
    const response = await fetch(`https://api.github.com/repos/${repository}${path}`, { headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' }, redirect: path.endsWith('/zip') ? 'follow' : 'error', signal: AbortSignal.timeout(60_000) })
    if (!response.ok) throw new Error(`Actions API ${response.status}`)
    return response
  }
  const download = async (id: number) => {
    const response = await request(`/actions/artifacts/${id}/zip`)
    const advertised = response.headers.get('content-length')
    if (advertised !== null && Number(advertised) > maxZipBytes) throw new Error('Public artifact ZIP size limit')
    const reader = response.body?.getReader()
    if (!reader) throw new Error('Missing Public artifact ZIP body')
    const chunks: Buffer[] = []
    let total = 0
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        total += value.byteLength
        if (total > maxZipBytes) throw new Error('Public artifact ZIP size limit')
        chunks.push(Buffer.from(value))
      }
    } finally { reader.releaseLock() }
    if (advertised !== null && total !== Number(advertised)) throw new Error('Incomplete Public artifact ZIP download')
    return Buffer.concat(chunks, total)
  }
  const result = await consume(root, out, async path => (await request(path)).json(), download)
  console.log(JSON.stringify(result))
  }
}

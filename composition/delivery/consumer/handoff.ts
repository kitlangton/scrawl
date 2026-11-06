import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'
import { sourceDigest, type Owner, type Target } from '../core.ts'

const repository = 'tryapparent/apparent'
const maxArtifactBytes = 1024 * 1024 * 1024
type Run = { id: number; run_attempt: number; head_sha: string; head_branch: string; event: string; conclusion: string | null; path?: string }
type Job = { name: string; status: string; conclusion: string | null }
type Artifact = { id: number; name: string; expired: boolean }
type Admission = { reuse: boolean; reason: string; input_digest: string; prior_artifact_id: number | null; prior_source_sha: string | null; prior_run_id: number | null; prior_run_attempt: number | null }
type Api = {
  runs(owner: Owner, sha: string): Promise<Run[]>
  jobs(run: Run): Promise<Job[]>
  artifacts(run: Run): Promise<Artifact[]>
  download(artifactId: number): Promise<Buffer>
  attest(path: string, owner: Owner, target: Target, sha: string, runId: number, attempt: number): Promise<void>
}

const fail = (message: string): never => { throw new Error(message) }
const sha256 = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')
export function matchesInvocation(value: unknown, runId: number, attempt: number): boolean {
  if (typeof value !== 'string') return false
  try {
    const url = new URL(value)
    return url.origin === 'https://github.com' && !url.username && !url.password && !url.search && !url.hash
      && url.pathname === `/${repository}/actions/runs/${runId}/attempts/${attempt}`
  } catch { return false }
}
function receiptFor(zip: Buffer, owner: Owner, target: Target, admission: Admission, runId: number, attempt: number, expectedSourceDigest: string) {
  const tmp = join(process.env.TMPDIR || '/tmp', `apparent-artifact-${process.pid}-${Math.random().toString(16).slice(2)}.zip`)
  try {
    writeFileSync(tmp, zip, { mode: 0o600, flag: 'wx' })
    const inventory = execFileSync('unzip', ['-Z1', tmp], { encoding: 'utf8', maxBuffer: 4096, timeout: 30000 }).trim().split('\n').sort()
    if (JSON.stringify(inventory) !== JSON.stringify(['component.tar.gz', 'receipt.json'])) fail('Producer artifact has an unexpected ZIP inventory')
    const raw = execFileSync('unzip', ['-p', tmp, 'receipt.json'], { encoding: 'utf8', maxBuffer: 16384, timeout: 30000 })
    const receipt = JSON.parse(raw)
    const archive = execFileSync('unzip', ['-p', tmp, 'component.tar.gz'], { maxBuffer: maxArtifactBytes, timeout: 60000 })
    if (!Buffer.isBuffer(archive) || archive.length !== receipt.size || sha256(archive) !== receipt.sha256) fail('Producer receipt does not match component archive bytes')
    if (receipt.schema !== 1 || receipt.id !== owner.id || receipt.target !== target || receipt.stage !== owner.stage || receipt.sourceSha !== admission.prior_source_sha || receipt.inputDigest !== admission.input_digest || receipt.sourceDigest !== expectedSourceDigest || receipt.runId !== String(runId) || receipt.runAttempt !== String(attempt)) fail('Producer receipt identity does not match the admitted main run and source tree')
    if (owner.stage === 'website' && (typeof receipt.sourceDigest !== 'string' || typeof receipt.environmentDigest !== 'string')) fail('Website receipt is missing source/environment identity')
    return { archive, receipt }
  } finally { rmSync(tmp, { force: true }) }
}

function validateTarInventory(archive: Buffer) {
  const tmp = join(process.env.TMPDIR || '/tmp', `apparent-component-${process.pid}-${Math.random().toString(16).slice(2)}.tar.gz`)
  try {
    writeFileSync(tmp, archive, { mode: 0o600, flag: 'wx' })
    const listing = execFileSync('tar', ['-P', '-tzf', tmp], { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, timeout: 30000, stdio: ['ignore', 'pipe', 'pipe'] })
    const names = listing.split('\n').filter(Boolean)
    if (!names.length || names.length > 200_000) fail('Component archive inventory is empty or too large')
    const canonical = names.map(name => name.replace(/\/$/, ''))
    if (new Set(canonical).size !== canonical.length) fail('Component archive contains duplicate paths')
    for (const name of names) {
      if (name.includes('\\') || name.startsWith('/') || /^[A-Za-z]:/.test(name) || name.includes('\0') || name.includes('\n') || name.includes('\r') || name.replace(/\/$/, '').split('/').some(part => !part || part === '.' || part === '..')) fail('Component archive contains an unsafe path')
    }
    const verbose = execFileSync('tar', ['-P', '-tvzf', tmp], { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, timeout: 30000, stdio: ['ignore', 'pipe', 'pipe'] })
    const entries = verbose.split('\n').filter(Boolean)
    if (entries.length !== names.length || entries.some(line => !/^[d-][rwxStTs-]{9}/.test(line))) fail('Component archive contains links or unsupported filesystem entries')
  } finally { rmSync(tmp, { force: true }) }
}

function cacheRoot() {
  const base = process.env.XDG_CACHE_HOME || join(homedir(), '.cache')
  const path = resolve(base, 'apparent', 'delivery')
  mkdirSync(path, { recursive: true, mode: 0o700 })
  if (lstatSync(resolve(base)).isSymbolicLink() || lstatSync(resolve(base, 'apparent')).isSymbolicLink() || lstatSync(path).isSymbolicLink()) fail('Delivery cache path cannot contain symlinks')
  chmodSync(path, 0o700)
  return path
}
function atomicCache(root: string, key: string, archive: Buffer, receipt: unknown) {
  const dir = join(root, key)
  if (existsSync(dir)) {
    try {
      if (lstatSync(dir).isSymbolicLink()) fail('Cached artifact entry is a symlink')
      const component = join(dir, 'component.tar.gz')
      const savedReceipt = join(dir, 'receipt.json')
      if (lstatSync(component).isFile() && lstatSync(savedReceipt).isFile() && sha256(readFileSync(component)) === sha256(archive) && readFileSync(savedReceipt, 'utf8') === JSON.stringify(receipt) + '\n') return dir
    } catch { /* A malformed entry is replaced after fresh network verification. */ }
    rmSync(dir, { recursive: true, force: true })
  }
  const lock = `${dir}.lock`
  let locked = false
  for (let i = 0; i < 200; i++) {
    try { mkdirSync(lock, { mode: 0o700 }); locked = true; break }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
      if (existsSync(dir)) return atomicCache(root, key, archive, receipt)
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 25)
    }
  }
  if (!locked) fail('Timed out waiting for another artifact handoff to publish its cache entry')
  const temp = `${dir}.tmp-${process.pid}-${Math.random().toString(16).slice(2)}`
  try {
    mkdirSync(temp, { mode: 0o700 })
    writeFileSync(join(temp, 'component.tar.gz'), archive, { mode: 0o600, flag: 'wx' })
    writeFileSync(join(temp, 'receipt.json'), JSON.stringify(receipt) + '\n', { mode: 0o600, flag: 'wx' })
    renameSync(temp, dir)
    return dir
  } finally { rmSync(temp, { recursive: true, force: true }); rmSync(lock, { recursive: true, force: true }) }
}

export async function handoff(root: string, owner: Owner, target: Target, sourceSha: string, api: Api, admissionFn: (runs: Run[], jobsFor: (run: Run) => Job[], artifactsFor: (run: Run) => Artifact[], exactInputDigest: string) => Admission, output?: string) {
  if (!/^[0-9a-f]{40}$/.test(sourceSha)) fail('Producer commit must be a full 40-character SHA')
  if (!owner.targets.includes(target)) fail(`${owner.id} does not produce target ${target}; choose one of: ${owner.targets.join(', ')}`)
  try { execFileSync('git', ['cat-file', '-e', `${sourceSha}^{commit}`], { cwd: root, stdio: 'ignore' }) }
  catch { fail(`Producer commit ${sourceSha} is unavailable locally; fetch that commit, then retry with the same explicit SHA`) }
  const runs = await api.runs(owner, sourceSha)
  // The existing admission function owns the policy; load exact-run evidence for its callbacks.
  const matchedRuns = runs.filter(run => run.head_sha === sourceSha)
  const jobMap = new Map<number, Job[]>(), artifactMap = new Map<number, Artifact[]>()
  for (const run of matchedRuns) { jobMap.set(run.id, await api.jobs(run)); artifactMap.set(run.id, await api.artifacts(run)) }
  const prefix = `stream-${owner.id}-${target}-`
  const candidates = matchedRuns.flatMap(run => (artifactMap.get(run.id) ?? []).filter(artifact => artifact.name.startsWith(prefix) && /^[0-9a-f]{64}$/.test(artifact.name.slice(prefix.length))).map(artifact => ({ run, artifact, digest: artifact.name.slice(prefix.length) })))
  if (candidates.length !== 1 || candidates[0].artifact.expired) fail(`No single unexpired ${owner.id}/${target} artifact exists on producer commit ${sourceSha}; confirm its exact main workflow completed successfully and retained the artifact.`)
  const candidate = candidates[0]
  const admitted = admissionFn(runs, run => jobMap.get(run.id) ?? [], run => artifactMap.get(run.id) ?? [], candidate.digest)
  const artifactId = admitted.prior_artifact_id, runId = admitted.prior_run_id, runAttempt = admitted.prior_run_attempt
  if (!admitted.reuse) fail(`No eligible ${owner.id}/${target} artifact for ${sourceSha}: ${admitted.reason}. Required: successful canonical main producer workflow, owner job, Delivery contracts and runtime audit, matching source fingerprint, and an unexpired exact artifact.`)
  if (artifactId === null || runId === null || runAttempt === null) throw new Error('Delivery admission omitted a required producer identity')
  if (artifactId !== candidate.artifact.id || runId !== candidate.run.id) fail('Delivery admission selected a different artifact than the explicit producer commit')
  const expectedSourceDigest = sourceDigest(root, owner, sourceSha)
  const cacheDir = cacheRoot()
  const identity = createHash('sha256').update(JSON.stringify([repository, owner.id, target, sourceSha, admitted.input_digest])).digest('hex')
  const identityLock = join(cacheDir, `${identity}.lock`)
  let locked = false
  for (let i = 0; i < 400; i++) {
    try { mkdirSync(identityLock, { mode: 0o700 }); locked = true; break }
    catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
      await new Promise(resolve => setTimeout(resolve, 25))
    }
  }
  if (!locked) fail('Timed out waiting for another artifact handoff to finish')
  try {
  let archive: Buffer | undefined
  let receipt: any
  let cacheHit = false
  const indexPath = join(cacheDir, `${identity}.json`)
  try {
    const index = JSON.parse(readFileSync(indexPath, 'utf8'))
    if (!/^[0-9a-f]{64}$/.test(index.cacheKey)) throw new Error('Bad cache index')
    const entry = join(cacheDir, index.cacheKey)
    const componentPath = join(entry, 'component.tar.gz'), receiptPath = join(entry, 'receipt.json')
    const entryStat = lstatSync(entry), componentStat = lstatSync(componentPath), receiptStat = lstatSync(receiptPath)
    if (!entryStat.isDirectory() || entryStat.isSymbolicLink() || [componentStat, receiptStat].some(stat => !stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || (stat.mode & 0o077) !== 0)) throw new Error('Cached component entry is not private regular data')
    archive = readFileSync(componentPath)
    receipt = JSON.parse(readFileSync(receiptPath, 'utf8'))
    if (archive.length !== receipt.size || sha256(archive) !== receipt.sha256 || receipt.schema !== 1 || receipt.id !== owner.id || receipt.target !== target || receipt.sourceSha !== sourceSha || receipt.inputDigest !== admitted.input_digest || receipt.sourceDigest !== expectedSourceDigest || receipt.runId !== String(runId) || receipt.runAttempt !== String(runAttempt)) throw new Error('Cached component is corrupt or mismatched')
    cacheHit = true
  } catch { archive = undefined; receipt = undefined; rmSync(indexPath, { force: true }) }
  if (!archive) {
    const zip = await api.download(artifactId)
    if (zip.length > maxArtifactBytes) fail('Producer artifact exceeds the 1 GiB safety limit')
    const parsed = receiptFor(zip, owner, target, admitted, runId, runAttempt, expectedSourceDigest)
    archive = parsed.archive; receipt = parsed.receipt
  }
  const verifiedArchive = archive
  if (!verifiedArchive) throw new Error('Verified component archive is unavailable')
  const temporaryArchive = join(process.env.TMPDIR || '/tmp', `apparent-attest-${process.pid}-${Math.random().toString(16).slice(2)}.tar.gz`)
  try {
    writeFileSync(temporaryArchive, verifiedArchive, { mode: 0o600, flag: 'wx' })
    await api.attest(temporaryArchive, owner, target, sourceSha, runId, runAttempt)
  } finally { rmSync(temporaryArchive, { force: true }) }
  validateTarInventory(verifiedArchive)
  const key = createHash('sha256').update(JSON.stringify([repository, owner.id, target, sourceSha, admitted.input_digest, receipt.sha256])).digest('hex')
  const cache = atomicCache(cacheDir, key, verifiedArchive, receipt)
  const indexTemp = `${indexPath}.tmp-${process.pid}`
  writeFileSync(indexTemp, JSON.stringify({ cacheKey: key }) + '\n', { mode: 0o600 })
  renameSync(indexTemp, indexPath)
  const destination = output ? resolve(output) : cache
  if (output) {
    mkdirSync(destination, { recursive: true, mode: 0o700 })
    if (lstatSync(destination).isSymbolicLink()) fail('Consumer destination cannot be a symlink')
    if (destination !== cache && readdirSync(destination).length) fail('Consumer destination must be empty')
    writeFileSync(join(destination, 'component.tar.gz'), readFileSync(join(cache, 'component.tar.gz')), { mode: 0o600, flag: 'wx' })
    writeFileSync(join(destination, 'receipt.json'), readFileSync(join(cache, 'receipt.json')), { mode: 0o600, flag: 'wx' })
  }
  return { schema: 1, repository, owner: owner.id, target, sourceSha, runId, runAttempt, artifactId, inputDigest: admitted.input_digest, componentSha256: receipt.sha256, cacheKey: key, cacheHit, componentPath: join(destination, 'component.tar.gz'), receiptPath: join(destination, 'receipt.json') }
  } finally { rmSync(identityLock, { recursive: true, force: true }) }
}

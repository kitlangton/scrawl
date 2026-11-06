import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { cpSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join, relative, resolve, sep } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { chunkPath, movie } from '../../lib/pages-video-metadata.ts'

export const ACCOUNT = 'a54dcebead3b8ad688f891da0ab54d1f'
export const PROJECT = 'apparent-capture'
export const ROOT = resolve(import.meta.dirname, '../../../..')
const MAX_ASSET = 25 * 1024 * 1024
const API_BASE = 'https://api.cloudflare.com/client/v4'
export type Inventory = Record<string, string>
export type Manifest = { schema: 1; revision: string; files: Inventory; movie: { path: string; sha256: string } }
export type PublisherDeps = { fetch?: typeof fetch; api?: (path: string) => Promise<any>; publicationNeeded?: (repo: string, sha: string, published: string | null) => Promise<boolean>; sleep?: (ms: number) => Promise<void>; root?: string; deploy?: (cwd: string, env: NodeJS.ProcessEnv, sha: string) => void }

export class SupersededMain extends Error {}
export class CanonicalPropagationPending extends Error {}
export class ReadbackTransferIncomplete extends Error {}

const digest = (data: Uint8Array | string) => createHash('sha256').update(data).digest('hex')
const regularTree = (root: string): Inventory => {
  if (lstatSync(root).isSymbolicLink() || !lstatSync(root).isDirectory()) throw new Error('Artifact root must be a regular directory')
  const result: Inventory = {}
  const walk = (dir: string) => {
    for (const name of readdirSync(dir).sort()) {
      const path = join(dir, name), info = lstatSync(path)
      if (info.isSymbolicLink() || (!info.isDirectory() && !info.isFile())) throw new Error('Artifact contains a symlink or nonregular file')
      if (info.isDirectory()) walk(path)
      else {
        if (info.size > MAX_ASSET) throw new Error('Artifact contains an oversized asset')
        result[relative(root, path).split(sep).join('/')] = digest(readFileSync(path))
      }
    }
  }
  walk(root)
  return Object.fromEntries(Object.entries(result).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
}

async function ghCompare(path: string): Promise<{ status: string }> {
  // Comparison patches can exceed the subprocess buffer; ordering needs only
  // the status. Project it inside gh rather than buffering the full diff here.
  const result = spawnSync('gh', ['api', path, '--jq', '.status'], { encoding: 'utf8' })
  if (result.status !== 0) throw new Error(result.stderr || result.error?.message || 'GitHub comparison failed')
  const status = result.stdout.trim()
  if (!['ahead', 'identical', 'behind', 'diverged'].includes(status)) throw new Error('Invalid GitHub comparison status')
  return { status }
}
export async function publicationNeeded(repo: string, sha: string, published: string | null, compare = ghCompare): Promise<boolean> {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) throw new Error('Invalid repository')
  for (const value of [sha, published]) if (value !== null && !/^[0-9a-f]{40}$/.test(value)) throw new Error('Unknown publication revision; refusing to guess ordering')
  if (!['ahead', 'identical'].includes((await compare(`repos/${repo}/compare/${sha}...main`)).status)) throw new Error('Candidate is no longer on main')
  if (published === null) return true
  const status = (await compare(`repos/${repo}/compare/${published}...${sha}`)).status
  if (status === 'ahead') return true
  if (status === 'identical' || status === 'behind') return false
  throw new Error('Published revision and candidate diverge; refusing publication')
}

export function inventory(root: string): Inventory { return regularTree(root) }
const sameInventory = (left: Inventory, right: Inventory) => {
  const keys = Object.keys(left)
  return keys.length === Object.keys(right).length && keys.every(key => left[key] === right[key])
}
// Pages receives only the Public site output. Keep this path contract here as
// a second admission boundary: a matching checksum manifest cannot authorize
// Connect, hosted app, or arbitrary files into the shared Pages project.
function isPublicOutputPath(name: string): boolean {
  if (name === 'wrangler.jsonc') return true
  if (!name.startsWith('dist-apparent/')) return false
  const path = name.slice('dist-apparent/'.length)
  if (['pond/apparent-qr.svg', 'index.html', '404.html', '_routes.json', '_worker.js', '_headers', '_redirects', 'robots.txt', 'sitemap.xml', 'icon.svg', 'apparent-wordmark.svg', 'social-preview-v1.png', 'Apparent-ghost-demo-v4-lilac-poster.jpg', 'HostGrotesk-Variable.ttf', 'footer-theatre.webp', 'frog-air-cohesive.png', 'frog-lounge-cohesive.png', 'frog-rest-cohesive.png', 'frog-wave-cohesive.png', 'lily-clean.png', 'olivia-dog-cutout.png', 'simon-cutout.png'].includes(path)) return true
  if (movie.chunks.some(chunk => path === chunkPath(chunk).slice(1))) return true
  if (/(?:^|[\/._-])(connect|hosted)(?:[\/._-]|$)/i.test(path)) return false
  return /^(assets\/[^/]+\.(?:js|css)|feature-demos\/(?:anywhere|handoff|how-it-works|timeline)\/(?:index\.html|\.local-media\/[^/]+\.(?:png|jpe?g|webp|mp4))|pond\/(?:belief\/[^/]+\.(?:png|jpe?g|webp)|[^/]+\.(?:png|jpe?g|webp))|team\/[^/]+\.(?:svg|png|jpe?g|webp))$/.test(path)
}
export function verifyArtifact(sha: string, artifact: string): Manifest {
  const actual = inventory(artifact)
  const manifest = JSON.parse(readFileSync(join(artifact, 'manifest.json'), 'utf8')) as Manifest
  delete actual['manifest.json']
  if (manifest.schema !== 1 || manifest.revision !== sha) throw new Error('Artifact revision mismatch')
  if (!sameInventory(actual, manifest.files)) throw new Error('Artifact checksum/inventory mismatch')
  if (Object.keys(actual).some(name => !isPublicOutputPath(name))) throw new Error('Unexpected artifact input')
  const headers = readFileSync(join(artifact, 'dist-apparent/_headers'), 'utf8')
  if (headers !== '/*\n  Cache-Control: no-cache, no-transform\n  X-Apparent-Publisher: cloudflare-pages\n' || readFileSync(join(artifact, 'dist-apparent/_redirects'), 'utf8') !== '') throw new Error('Invalid Public Pages metadata')
  const routes = JSON.parse(readFileSync(join(artifact, 'dist-apparent/_routes.json'), 'utf8'))
  if (JSON.stringify(routes) !== JSON.stringify({ version: 1, include: [movie.path], exclude: [] })) throw new Error('Invalid Public video routes')
  if (manifest.movie?.path !== movie.path || manifest.movie.sha256 !== movie.sha256) throw new Error('Invalid Public movie manifest')
  const wholeMovie = createHash('sha256')
  for (const chunk of movie.chunks) {
    const path = chunkPath(chunk).slice(1), file = join(artifact, 'dist-apparent', path)
    if (actual['dist-apparent/' + path] !== chunk.sha256 || statSync(file).size !== chunk.size) throw new Error('Invalid approved Public movie chunk')
    wholeMovie.update(readFileSync(file))
  }
  if (wholeMovie.digest('hex') !== movie.sha256) throw new Error('Invalid approved Public movie reconstruction')
  return manifest
}
export function seal(sha: string, root = ROOT): Manifest {
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error('Invalid artifact revision')
  const artifact = join(root, '.build/website')
  mkdirSync(join(root, '.build'), { recursive: true })
  mkdirSync(artifact, { recursive: false })
  cpSync(join(root, 'public/site/dist-apparent'), join(artifact, 'dist-apparent'), { recursive: true, errorOnExist: true, force: false })
  writeFileSync(join(artifact, 'wrangler.jsonc'), readFileSync(join(root, 'public/site/wrangler.pages.jsonc')))
  const files = inventory(artifact)
  for (const required of ['dist-apparent/index.html', 'dist-apparent/404.html', 'dist-apparent/_routes.json', 'dist-apparent/_worker.js']) if (!(required in files)) throw new Error('Incomplete Pages artifact: ' + required)
  const manifest: Manifest = { schema: 1, revision: sha, files, movie: { path: movie.path, sha256: movie.sha256 } }
  // Preserve Python's recursively sorted, indented JSON representation.
  writeFileSync(join(artifact, 'manifest.json'), JSON.stringify({ files: Object.fromEntries(Object.entries(files).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)), movie: { path: manifest.movie.path, sha256: manifest.movie.sha256 }, revision: sha, schema: 1 }, null, 2) + '\n')
  verifyArtifact(sha, artifact)
  return manifest
}

export function differenceMetadata(expected: Uint8Array, actual: Uint8Array) {
  let prefix = 0, limit = Math.min(expected.length, actual.length)
  while (prefix < limit && expected[prefix] === actual[prefix]) prefix++
  let suffix = 0
  while (suffix < limit - prefix && expected[expected.length - suffix - 1] === actual[actual.length - suffix - 1]) suffix++
  const changed = actual.subarray(prefix, actual.length - suffix)
  const markers: Record<string, string> = { speculationrules: 'type="speculationrules"', cloudflare_beacon: 'static.cloudflareinsights.com', cloudflare_challenge: '/cdn-cgi/challenge-platform/', rocket_loader: 'rocket-loader.min.js', email_obfuscation: 'email-decode.min.js' }
  const text = Buffer.from(changed).toString('latin1')
  return { common_prefix_bytes: prefix, common_suffix_bytes: suffix, expected_changed_bytes: expected.length - prefix - suffix, actual_changed_bytes: changed.length, actual_changed_sha256: digest(changed), edge_markers: Object.fromEntries(Object.entries(markers).map(([key, marker]) => [key, text.includes(marker)])) }
}

async function defaultApi(path: string, fetcher: typeof fetch): Promise<any> {
  const token = process.env.WEBSITE_PAGES_API_TOKEN
  if (!token) throw new Error('WEBSITE_PAGES_API_TOKEN is required')
  const response = await fetcher(API_BASE + path, { headers: { Authorization: 'Bearer ' + token }, signal: AbortSignal.timeout(60_000) })
  if (!response.ok) throw Object.assign(new Error(`Cloudflare API HTTP ${response.status}`), { status: response.status })
  const value = await response.json() as any
  if (!value.success) throw new Error('Pages readback failed')
  return value.result
}
async function currentMain(sha: string, deps: PublisherDeps): Promise<void> {
  const project = await (deps.api ?? (path => defaultApi(path, deps.fetch ?? fetch)))(`/accounts/${ACCOUNT}/pages/projects/${PROJECT}`)
  if (!project || !Object.hasOwn(project, 'canonical_deployment')) throw new Error('Missing canonical deployment identity')
  const deployment = project.canonical_deployment
  const published = deployment == null ? null : deployment.deployment_trigger.metadata.commit_hash
  if (!(await (deps.publicationNeeded ?? publicationNeeded)(process.env.GITHUB_REPOSITORY!, sha, published))) throw new SupersededMain('An equal or newer website revision is already deployed')
}

function responseMeta(headers: Headers, names: string[]) { return Object.fromEntries(names.map(name => [name.toLowerCase().replaceAll('-', '_'), headers.get(name)])) }
export async function verify(sha: string, packagePath: string, deps: PublisherDeps = {}) {
  const fetcher = deps.fetch ?? fetch
  const api = deps.api ?? (path => defaultApi(path, fetcher))
  const project = await api(`/accounts/${ACCOUNT}/pages/projects/${PROJECT}`)
  const deployment = project.canonical_deployment
  if (deployment.latest_stage.status !== 'success' || deployment.deployment_trigger.metadata.commit_hash !== sha) throw new Error('Pages production is not the checked revision')
  const manifest = verifyArtifact(sha, resolve(packagePath, '..'))
  const paths: [string, string][] = [['index.html', '/']]
  for (const name of Object.keys(manifest.files)) if (name.startsWith('dist-apparent/assets/') && /\.(js|css)$/.test(name)) { const path = name.slice('dist-apparent/'.length); paths.push([path, '/' + path]) }
  const origins = [...new Set([deployment.url, 'https://app.tryapparent.com'])]
  for (let oi = 0; oi < origins.length; oi++) {
    const origin = origins[oi], mismatch = (message: string) => oi ? new CanonicalPropagationPending(message) : new Error(message)
    for (const [path, route] of paths) {
      const response = await fetcher(origin + route, { headers: { 'Cache-Control': 'no-cache', 'User-Agent': 'Apparent-CD/1.0' }, signal: AbortSignal.timeout(60_000) })
      if (!response.ok) throw Object.assign(new Error(`HTTP ${response.status}`), { status: response.status })
      const body = new Uint8Array(await response.arrayBuffer()), headers = response.headers
      const responseMetaValue = responseMeta(headers, ['Content-Encoding', 'Content-Type', 'CF-Cache-Status', 'Age', 'CF-Ray'])
      if (headers.get('X-Apparent-Publisher') !== 'cloudflare-pages') throw new Error('Missing Pages publisher header')
      const actualHash = digest(body), expectedHash = manifest.files['dist-apparent/' + path]
      if (actualHash !== expectedHash) {
        const expected = readFileSync(join(packagePath, path))
        const detail = { ...responseMetaValue, actual_sha256: actualHash, expected_sha256: expectedHash, actual_bytes: body.length, expected_bytes: statSync(join(packagePath, path)).size, ...differenceMetadata(expected, body) }
        throw mismatch(`Pages served different bytes: ${origin}${route} ${JSON.stringify(detail)}`)
      }
    }
    const media = manifest.movie
    const response = await fetcher(origin + media.path, { headers: { 'Cache-Control': 'no-cache', 'User-Agent': 'Apparent-CD/1.0' }, signal: AbortSignal.timeout(60_000) })
    if (!response.ok) throw Object.assign(new Error(`HTTP ${response.status}`), { status: response.status })
    const meta = { status: response.status, ...responseMeta(response.headers, ['Content-Length', 'Content-Range', 'Content-Encoding', 'CF-Cache-Status', 'Age', 'CF-Ray']) }
    if (response.headers.get('Content-Type') !== 'video/mp4') throw new Error('Pages movie is unavailable')
    const lengthHeader = response.headers.get('Content-Length')
    const expectedTransferBytes = lengthHeader === null ? null : Number(lengthHeader)
    const hash = createHash('sha256')
    let actualBytes = 0
    try {
      if (response.body) {
        const reader = response.body.getReader()
        try {
          while (true) {
            const { done, value } = await reader.read()
            if (done) break
            hash.update(value)
            actualBytes += value.byteLength
          }
        } finally { reader.releaseLock() }
      }
    } catch (error) {
      if (expectedTransferBytes !== null && actualBytes < expectedTransferBytes) {
        const detail = { ...meta, actual_sha256: hash.copy().digest('hex'), expected_sha256: media.sha256, actual_bytes: actualBytes }
        throw new ReadbackTransferIncomplete(`Pages movie transfer incomplete: ${origin}${media.path} ${JSON.stringify(detail)}`, { cause: error })
      }
      throw error
    }
    const actualHash = hash.digest('hex')
    const detail = { ...meta, actual_sha256: actualHash, expected_sha256: media.sha256, actual_bytes: actualBytes }
    if (expectedTransferBytes !== null && actualBytes !== expectedTransferBytes) throw new ReadbackTransferIncomplete(`Pages movie transfer incomplete: ${origin}${media.path} ${JSON.stringify(detail)}`)
    if (actualHash !== media.sha256) throw mismatch(`Pages served different movie bytes: ${origin}${media.path} ${JSON.stringify(detail)}`)
  }
  return { revision: sha, project: PROJECT, deployment_id: deployment.id, url: deployment.url, public_url: 'https://app.tryapparent.com', manifest_sha256: digest(readFileSync(join(resolve(packagePath, '..'), 'manifest.json'))) }
}
export async function verifyReady(sha: string, packagePath: string, deps: PublisherDeps = {}) {
  const sleep = deps.sleep ?? ((ms: number) => new Promise(resolveSleep => setTimeout(resolveSleep, ms)))
  for (let attempt = 0; attempt < 5; attempt++) {
    try { return await verify(sha, packagePath, deps) }
    catch (error) {
      const retry = error instanceof CanonicalPropagationPending || error instanceof ReadbackTransferIncomplete || ([404, 500, 502, 503, 504].includes((error as any).status))
      if (!retry || attempt === 4) throw error
      await sleep(2 ** attempt * 1000)
    }
  }
  throw new Error('Unreachable readiness retry state')
}

export async function main(deps: PublisherDeps = {}) {
  const root = deps.root ?? ROOT, sha = process.env.GITHUB_SHA ?? ''
  if (!/^[0-9a-f]{40}$/.test(sha) || process.env.GITHUB_REF !== 'refs/heads/main') throw new Error('Only exact main revisions may publish')
  if (process.env.GITHUB_REPOSITORY !== 'tryapparent/apparent' ||
      process.env.GITHUB_WORKFLOW_REF !== 'tryapparent/apparent/.github/workflows/stream-public.yml@refs/heads/main' ||
      process.env.GITHUB_JOB !== 'deploy') throw new Error('Public publication requires the canonical value-stream deploy job')
  const artifact = join(root, '.build/website'), manifest = verifyArtifact(sha, artifact), packagePath = join(artifact, 'dist-apparent')
  if (!existsSync(join(packagePath, '404.html')) || !existsSync(join(packagePath, '_routes.json'))) throw new Error('Missing Public routing preparation')
  if (lstatSync(packagePath).isSymbolicLink() || Object.entries(inventory(packagePath)).some(([name]) => statSync(join(packagePath, name)).size > MAX_ASSET)) throw new Error('Package has a symlink or oversized asset')
  const token = process.env.WEBSITE_PAGES_API_TOKEN
  if (!token) throw new Error('WEBSITE_PAGES_API_TOKEN is required')
  const env = { ...process.env, CLOUDFLARE_API_TOKEN: token, CLOUDFLARE_ACCOUNT_ID: ACCOUNT, WRANGLER_SEND_METRICS: 'false' }
  const stage = mkdtempSync(join(tmpdir(), 'apparent-pages-'))
  try {
    cpSync(packagePath, join(stage, 'dist-apparent'), { recursive: true })
    writeFileSync(join(stage, 'wrangler.jsonc'), readFileSync(join(artifact, 'wrangler.jsonc')))
    if (!sameInventory(inventory(stage), manifest.files)) throw new Error('Staged artifact checksum mismatch')
    try { await currentMain(sha, deps) } catch (error) { if (error instanceof SupersededMain) { console.log('Skipping superseded main candidate; no website published'); return } throw error }
    if (deps.deploy) deps.deploy(stage, env, sha)
    else {
      const cli = join(root, 'public/site/node_modules/.bin/wrangler')
      const result = spawnSync(cli, ['pages', 'deploy', '--project-name', PROJECT, '--branch', 'main', '--commit-hash', sha, '--commit-dirty=false', '--no-bundle'], { cwd: stage, env, stdio: 'inherit' })
      if (result.status !== 0) throw new Error(`Wrangler deployment failed (${result.status})`)
    }
  } finally { rmSync(stage, { recursive: true, force: true }) }
  const receipt = await verifyReady(sha, packagePath, deps)
  writeFileSync(join(root, 'website-receipt.json'), JSON.stringify(receipt) + '\n')
  console.log(JSON.stringify(receipt))
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2)
  if (args.length === 2 && args[0] === 'seal') seal(args[1])
  else if (args.length === 0) await main()
  else throw new Error('Usage: publish-pages.ts [seal SHA]')
}

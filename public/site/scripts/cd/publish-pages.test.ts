import test from 'node:test'
import { spawnSync } from 'node:child_process'
import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { CanonicalPropagationPending, differenceMetadata, inventory, main, publicationNeeded, ReadbackTransferIncomplete, seal, verify, verifyArtifact, verifyReady, type PublisherDeps } from './publish-pages.ts'
import { movie } from '../../lib/pages-video-metadata.js'
import { preparePagesVideo } from '../pages/prepare-pages-video.mjs'

const sha = 'a'.repeat(40)
const approvedMovie = readFileSync(new URL('../../public/assets/media/Apparent-ghost-demo-v4-lilac.mp4', import.meta.url))
function fixture(root: string) {
  const files: Record<string, string> = {
    'public/site/dist-apparent/index.html': 'source homepage', 'public/site/dist-apparent/404.html': 'not found',
    'public/site/dist-apparent/_routes.json': JSON.stringify({ version: 1, include: [movie.path], exclude: [] }) + '\n', 'public/site/dist-apparent/_worker.js': 'compiled worker',
    'public/site/dist-apparent/assets/index-test.js': 'compiled source',
    'public/site/dist-apparent/_headers': '/*\n  Cache-Control: no-cache, no-transform\n  X-Apparent-Publisher: cloudflare-pages\n',
    'public/site/dist-apparent/_redirects': '',
    'public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4': 'source must not reach publisher',
    'public/site/functions/movie.js': 'source must not reach publisher', 'public/site/wrangler.pages.jsonc': '{"name":"apparent-capture"}',
    'public/site/wrangler.jsonc': 'unrelated Worker', '.env': 'private sentinel',
  }
  for (const [name, data] of Object.entries(files)) { const path = join(root, name); mkdirSync(join(path, '..'), { recursive: true }); writeFileSync(path, data) }
  const output = join(root, 'public/site/dist-apparent')
  writeFileSync(join(output, movie.path.slice(1)), approvedMovie)
  preparePagesVideo(output)
  seal(sha, root)
  return join(root, '.build/website')
}
async function temp<T>(run: (root: string) => Promise<T> | T): Promise<T> {
  const root = mkdtempSync(join(tmpdir(), 'pages delivery test-'))
  try { return await run(root) } finally { rmSync(root, { recursive: true, force: true }) }
}
const project = (revision = sha) => ({ canonical_deployment: { id: 'deployment', url: 'https://deployment.apparent-capture.pages.dev', latest_stage: { status: 'success' }, deployment_trigger: { metadata: { commit_hash: revision } } } })

 test('difference metadata identifies edits without exposing response text', () => {
  const prefix = Buffer.from('<body>approved'), suffix = Buffer.from('</body>'), inserted = Buffer.from('<script type="speculationrules">private-body-sentinel</script>')
  const result = differenceMetadata(Buffer.concat([prefix, suffix]), Buffer.concat([prefix, inserted, suffix]))
  assert.equal(result.common_prefix_bytes, prefix.length + 1)
  assert.equal(result.expected_changed_bytes, 0)
  assert.equal(result.actual_changed_bytes, inserted.length)
  assert.equal(result.edge_markers.speculationrules, true)
  assert.equal(JSON.stringify(result).includes('private-body-sentinel'), false)
  for (const [expected, actual, sizes] of [[Buffer.from('abc'), Buffer.from('axc'), [1, 1]], [Buffer.from('abc'), Buffer.from('ac'), [1, 0]], [Buffer.alloc(0), Buffer.from('abc'), [0, 3]], [Buffer.from('abc'), Buffer.from('abc'), [0, 0]]] as const) {
    const item = differenceMetadata(expected, actual)
    assert.deepEqual([item.expected_changed_bytes, item.actual_changed_bytes], sizes)
  }
})

test('publication admission requires current main and checks deployed revision ordering', async () => {
  const calls: string[] = []
  const fetchCompare = async (path: string) => { calls.push(path); return { status: path.includes('...main') ? 'identical' : 'ahead' } }
  assert.equal(await publicationNeeded('example/project', 'c'.repeat(40), 'b'.repeat(40), fetchCompare), true)
  assert.deepEqual(calls, ['repos/example/project/compare/' + 'c'.repeat(40) + '...main', 'repos/example/project/compare/' + 'b'.repeat(40) + '...' + 'c'.repeat(40)])
  assert.equal(await publicationNeeded('example/project', sha, sha, async () => ({ status: 'identical' })), false)
  await assert.rejects(publicationNeeded('example/project', sha, null, async () => ({ status: 'ahead' })), /no longer current main/)
  await assert.rejects(publicationNeeded('bad repo', sha, null), /Invalid repository/)
  await assert.rejects(publicationNeeded('example/project', sha, null, async () => ({ status: 'behind' })), /no longer on main/)
  let divergeCall = 0
  await assert.rejects(publicationNeeded('example/project', sha, 'b'.repeat(40), async () => ({ status: ++divergeCall === 1 ? 'identical' : 'diverged' })), /diverge/)
})

test('real comparison command projects status before buffering a large GitHub response', () => temp(async root => {
  const tool = join(root, 'gh'), calls = join(root, 'gh-calls')
  writeFileSync(tool, `#!/bin/sh
printf '%s\\n' "$*" >> "$COMPARISON_CALLS"
if [ "$COMPARISON_FAIL" = true ]; then exit 1; fi
case "$*" in
  *...main*) printf '%s\\n' "\${COMPARISON_STATUS:-identical}" ;;
  *'--jq .status'*) printf '%s\\n' "ahead" ;;
  *) printf '{"status":"ahead","files":[{"patch":"'; head -c 1200000 /dev/zero | tr '\\000' x; printf '"}]}' ;;
esac
`, { mode: 0o700 })
  const prior = { PATH: process.env.PATH, COMPARISON_CALLS: process.env.COMPARISON_CALLS,
    COMPARISON_FAIL: process.env.COMPARISON_FAIL, COMPARISON_STATUS: process.env.COMPARISON_STATUS }
  Object.assign(process.env, { PATH: root + ':' + process.env.PATH, COMPARISON_CALLS: calls })
  delete process.env.COMPARISON_FAIL; delete process.env.COMPARISON_STATUS
  try {
    // The same unprojected CLI response exceeds spawnSync's default 1 MiB.
    const unprojected = spawnSync(tool, ['api', 'repos/example/project/compare/base...head'], { encoding: 'utf8' })
    assert.equal((unprojected.error as NodeJS.ErrnoException)?.code, 'ENOBUFS')
    assert.equal(await publicationNeeded('example/project', sha, 'b'.repeat(40)), true)
    assert.equal(readFileSync(calls, 'utf8').split('\n').filter(line => line.includes('--jq .status')).length, 2)
    process.env.COMPARISON_STATUS = 'behind'
    await assert.rejects(publicationNeeded('example/project', sha, null), /no longer on main/)
    process.env.COMPARISON_STATUS = 'unexpected'
    await assert.rejects(publicationNeeded('example/project', sha, null))
    process.env.COMPARISON_FAIL = 'true'
    await assert.rejects(publicationNeeded('example/project', sha, null), /GitHub comparison failed/)
  } finally {
    for (const [key, value] of Object.entries(prior)) if (value === undefined) delete process.env[key]; else process.env[key] = value
  }
}))

test('sealed artifact rejects revision changes, content edits, extra inputs, symlinks and reuse', () => temp(root => {
  const artifact = fixture(root)
  assert.equal(verifyArtifact(sha, artifact).revision, sha)
  assert.throws(() => verifyArtifact('b'.repeat(40), artifact), /revision/)
  const file = join(artifact, 'dist-apparent/index.html'), original = readFileSync(file)
  writeFileSync(file, 'tampered'); assert.throws(() => verifyArtifact(sha, artifact), /checksum/); writeFileSync(file, original)
  const extra = join(artifact, 'dist-apparent/extra.js'); writeFileSync(extra, 'unexpected'); assert.throws(() => verifyArtifact(sha, artifact), /checksum/); rmSync(extra)
  const manifestPath = join(artifact, 'manifest.json'), originalManifest = readFileSync(manifestPath)
  for (const path of ['connect/index.html', 'hosted/index.html', 'hosted/app.js', 'assets/connect-private.js', 'pond/unapproved.svg']) {
    const file = join(artifact, 'dist-apparent', path); mkdirSync(join(file, '..'), { recursive: true }); writeFileSync(file, 'forbidden')
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
    manifest.files['dist-apparent/' + path] = inventory(artifact)['dist-apparent/' + path]
    writeFileSync(manifestPath, JSON.stringify(manifest))
    assert.throws(() => verifyArtifact(sha, artifact), /Unexpected artifact input/, `forged manifest must not admit ${path}`)
    rmSync(file); writeFileSync(manifestPath, originalManifest)
  }
  symlinkSync(root, extra, 'dir'); assert.throws(() => inventory(artifact), /symlink/); rmSync(extra)
  assert.throws(() => seal(sha, root), /EEXIST/)
}))

test('sealing admits the canonical frog QR asset', () => temp(root => {
  fixture(root)
  rmSync(join(root, '.build/website'), { recursive: true, force: true })
  const qr = readFileSync(new URL('../../public/assets/pond/apparent-qr.svg', import.meta.url))
  const pond = join(root, 'public/site/dist-apparent/pond')
  mkdirSync(pond)
  writeFileSync(join(pond, 'apparent-qr.svg'), qr)
  seal(sha, root)
  const artifact = join(root, '.build/website')
  assert.equal(verifyArtifact(sha, artifact).revision, sha)
  assert.deepEqual(readFileSync(join(artifact, 'dist-apparent/pond/apparent-qr.svg')), qr)
}))

test('sealing rejects Connect and hosted outputs from the Public build directory', () => temp(root => {
  fixture(root)
  rmSync(join(root, '.build/website'), { recursive: true, force: true })
  const website = join(root, 'public/site/dist-apparent')
  mkdirSync(join(website, 'connect'), { recursive: true }); writeFileSync(join(website, 'connect/index.html'), 'Connect')
  mkdirSync(join(website, 'hosted'), { recursive: true }); writeFileSync(join(website, 'hosted/index.html'), 'hosted app')
  assert.throws(() => seal(sha, root), /Unexpected artifact input/)
}))

test('readback binds revision before any public HTTP request', async () => {
  let requested = false
  await assert.rejects(verify('b'.repeat(40), '/unused', { api: async () => project(), fetch: async () => { requested = true; return new Response() } }), /checked revision/)
  assert.equal(requested, false)
})

test('readback verifies Public HTML, JS, movie and both origins without requesting Connect', () => temp(async root => {
  const artifact = fixture(root), packagePath = join(artifact, 'dist-apparent')
  const bodies: Record<string, string> = { '/': 'index.html', '/assets/index-test.js': 'assets/index-test.js' }
  const visited = new Set<string>()
  let badPath: string | null = null, badOrigin: string | null = null, shortMovie = 0, corruptMovie = false
  const fetcher: typeof fetch = async input => {
    const url = new URL(String(input)); visited.add(url.host)
    assert(!url.pathname.startsWith('/connect') && !url.pathname.startsWith('/hosted'), `Public publisher must not request ${url.pathname}`)
    const headers = new Headers({ 'X-Apparent-Publisher': 'cloudflare-pages', 'Cache-Control': 'no-store', 'X-Robots-Tag': 'noindex, nofollow', 'Content-Type': 'video/mp4', 'Set-Cookie': 'private-cookie-sentinel' })
    if (url.pathname === badPath && (badOrigin === null || url.host === badOrigin)) return new Response('unexpected response body sentinel', { headers })
    if (url.pathname.endsWith('.mp4')) {
      if (shortMovie > 0) {
        shortMovie--
        const broken = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(Buffer.from('checked mo')); controller.error(new TypeError('premature socket close')) } })
        return new Response(broken, { headers: new Headers({ ...Object.fromEntries(headers), 'Content-Length': '13' }) })
      }
      const movieBytes = corruptMovie ? Buffer.alloc(13, 'x') : approvedMovie
      headers.set('Content-Length', String(movieBytes.length)); return new Response(movieBytes, { headers })
    }
    return new Response(readFileSync(join(packagePath, bodies[url.pathname])), { headers })
  }
  const deps: PublisherDeps = { api: async () => project(), fetch: fetcher, sleep: async () => {} }
  assert.equal((await verify(sha, packagePath, deps)).deployment_id, 'deployment')
  assert.deepEqual(visited, new Set(['deployment.apparent-capture.pages.dev', 'app.tryapparent.com']))
  shortMovie = 1
  const sleeps: number[] = []
  assert.equal((await verifyReady(sha, packagePath, { ...deps, sleep: async ms => { sleeps.push(ms) } })).deployment_id, 'deployment')
  assert.deepEqual(sleeps, [1000])
  shortMovie = 5; sleeps.length = 0
  await assert.rejects(verifyReady(sha, packagePath, { ...deps, sleep: async ms => { sleeps.push(ms) } }), ReadbackTransferIncomplete)
  assert.deepEqual(sleeps, [1000, 2000, 4000, 8000])
  shortMovie = 0; corruptMovie = true; sleeps.length = 0
  await assert.rejects(verifyReady(sha, packagePath, { ...deps, sleep: async ms => { sleeps.push(ms) } }), /different movie/)
  assert.deepEqual(sleeps, [])
  corruptMovie = false
  for (const path of Object.keys(bodies)) {
    badPath = path
    await assert.rejects(verify(sha, packagePath, deps), error => error instanceof Error && error.message.includes('different bytes') && error.message.includes('actual_sha256') && !error.message.includes('unexpected response body sentinel') && !error.message.includes('private-cookie-sentinel'))
  }
  badPath = '/Apparent-ghost-demo-v4-lilac.mp4'
  await assert.rejects(verify(sha, packagePath, deps), error => error instanceof Error && ['actual_sha256', 'expected_sha256', 'actual_bytes', 'content_range', 'content_encoding', 'status'].every(key => error.message.includes(key)) && !error.message.includes('private-cookie-sentinel'))
  badPath = '/'; badOrigin = 'app.tryapparent.com'
  await assert.rejects(verify(sha, packagePath, deps), CanonicalPropagationPending)
  badPath = null; badOrigin = null
  const missingPublisherHeader: typeof fetch = async () => new Response('public html', { headers: { 'Content-Type': 'text/html' } })
  await assert.rejects(verify(sha, packagePath, { ...deps, fetch: missingPublisherHeader }), /Missing Pages publisher header/)
}))

test('publisher stages exact bytes, writes success receipt, skips superseded candidates, and fails closed', () => temp(async root => {
  const artifact = fixture(root)
  const movieBytes = approvedMovie
  rmSync(join(root, 'public/site'), { recursive: true })
  const oldEnv = { sha: process.env.GITHUB_SHA, ref: process.env.GITHUB_REF, token: process.env.WEBSITE_PAGES_API_TOKEN, repo: process.env.GITHUB_REPOSITORY, workflow: process.env.GITHUB_WORKFLOW_REF, job: process.env.GITHUB_JOB, event: process.env.GITHUB_EVENT_NAME }
  process.env.GITHUB_SHA = sha; process.env.GITHUB_REF = 'refs/heads/main'; process.env.WEBSITE_PAGES_API_TOKEN = 'synthetic'; process.env.GITHUB_REPOSITORY = 'tryapparent/apparent'; process.env.GITHUB_WORKFLOW_REF = 'tryapparent/apparent/.github/workflows/stream-public.yml@refs/heads/main'; process.env.GITHUB_JOB = 'deploy'; process.env.GITHUB_EVENT_NAME = 'workflow_dispatch'
  try {
    const staged: string[] = [], packagePath = join(artifact, 'dist-apparent')
    let apiCalls = 0
    const deps: PublisherDeps = {
      root, api: async () => ++apiCalls === 1 ? { canonical_deployment: null } : project(), publicationNeeded: async () => true,
      deploy: (cwd, env, suppliedSha) => {
        staged.push(cwd); assert.equal(suppliedSha, sha); assert.equal(env.CLOUDFLARE_API_TOKEN, 'synthetic')
        assert.equal(env.CLOUDFLARE_ACCOUNT_ID, 'a54dcebead3b8ad688f891da0ab54d1f')
        assert.deepEqual(readdirSync(cwd).sort(), ['dist-apparent', 'wrangler.jsonc'])
        assert.equal(readFileSync(join(cwd, 'wrangler.jsonc'), 'utf8'), '{"name":"apparent-capture"}')
        assert.equal(readFileSync(join(cwd, 'dist-apparent/_worker.js'), 'utf8'), 'compiled worker')
      },
      fetch: async input => {
        const url = new URL(String(input)), headers = new Headers({ 'X-Apparent-Publisher': 'cloudflare-pages', 'Cache-Control': 'no-store', 'X-Robots-Tag': 'noindex, nofollow', 'Content-Type': 'video/mp4' })
        if (url.pathname.endsWith('.mp4')) return new Response(movieBytes, { headers })
        const paths: Record<string, string> = { '/': 'index.html', '/assets/index-test.js': 'assets/index-test.js' }
        return new Response(readFileSync(join(packagePath, paths[url.pathname])), { headers })
      },
    }
    for (const [key, invalid] of [['GITHUB_REPOSITORY', 'fork/apparent'], ['GITHUB_WORKFLOW_REF', 'tryapparent/apparent/.github/workflows/web-delivery.yml@refs/heads/main'], ['GITHUB_WORKFLOW_REF', undefined], ['GITHUB_JOB', 'website'], ['GITHUB_EVENT_NAME', 'push'], ['GITHUB_EVENT_NAME', 'pull_request']]) {
      const original = process.env[key!]
      if (invalid === undefined) delete process.env[key!]; else process.env[key!] = invalid
      await assert.rejects(main(deps), /canonical manual deploy job/)
      if (original === undefined) delete process.env[key!]; else process.env[key!] = original
    }
    assert.equal(staged.length, 0, 'Other workflow contexts must fail before publication')
    await main(deps)
    const receipt = JSON.parse(readFileSync(join(root, 'website-receipt.json'), 'utf8'))
    assert.equal(receipt.revision, sha)
    assert.equal(receipt.deployment_id, 'deployment')
    assert.equal(staged.length, 1)
    assert.equal(existsSync(staged[0]), false)

    await assert.rejects(main({ ...deps, api: async () => ({}) }), /Missing canonical deployment identity/)
  await main({ ...deps, publicationNeeded: async () => false })
    assert.equal(staged.length, 1)

    process.env.GITHUB_REF = 'refs/heads/feature'
    await assert.rejects(main(deps), /Only exact main revisions/)
    process.env.GITHUB_REF = 'refs/heads/main'
    delete process.env.WEBSITE_PAGES_API_TOKEN
    await assert.rejects(main(deps), /WEBSITE_PAGES_API_TOKEN is required/)
    assert.equal(staged.length, 1)
    process.env.WEBSITE_PAGES_API_TOKEN = 'synthetic'
    writeFileSync(join(artifact, 'dist-apparent/index.html'), 'changed after verification')
    await assert.rejects(main(deps), /checksum/)
  } finally {
    for (const [key, value] of Object.entries({ GITHUB_SHA: oldEnv.sha, GITHUB_REF: oldEnv.ref, WEBSITE_PAGES_API_TOKEN: oldEnv.token, GITHUB_REPOSITORY: oldEnv.repo, GITHUB_WORKFLOW_REF: oldEnv.workflow, GITHUB_JOB: oldEnv.job, GITHUB_EVENT_NAME: oldEnv.event })) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
}))

test('readiness retries transient HTTP and propagation failures only', async () => {
  let calls = 0; const sleeps: number[] = []
  const deps: PublisherDeps = { api: async () => project(), fetch: async () => { calls++; if (calls === 1) return new Response('', { status: 503 }); return new Response('') }, sleep: async ms => { sleeps.push(ms) } }
  // Verify's API identity passes, then a temporary response is retried.
  await assert.rejects(verifyReady(sha, '/missing', deps))
  assert.deepEqual(sleeps, [])
  let attempt = 0
  const retryDeps: PublisherDeps = { ...deps, api: async () => { attempt++; throw Object.assign(new Error('temporary'), { status: 503 }) } }
  await assert.rejects(verifyReady(sha, '/missing', retryDeps), /temporary/)
  assert.deepEqual(sleeps, [1000, 2000, 4000, 8000])
  sleeps.length = 0
  let authCalls = 0
  const authDeps: PublisherDeps = { ...deps, api: async () => { authCalls++; throw Object.assign(new Error('unauthorized'), { status: 401 }) } }
  await assert.rejects(verifyReady(sha, '/missing', authDeps), /unauthorized/)
  assert.equal(authCalls, 1)
  assert.deepEqual(sleeps, [])
  await assert.rejects(publicationNeeded('example/project', sha, 'z'.repeat(40), async () => ({ status: 'identical' })), /Unknown publication revision/)
})

test('publisher CLI seals from a source path containing spaces', () => temp(root => {
  fixture(root)
  rmSync(join(root, '.build/website'), { recursive: true })
  const script = join(root, 'public/site/scripts/cd/publish-pages.ts')
  mkdirSync(join(script, '..'), { recursive: true })
  writeFileSync(script, readFileSync(new URL('./publish-pages.ts', import.meta.url)))
  const metadata = join(root, 'public/site/lib/pages-video-metadata.js')
  mkdirSync(join(metadata, '..'), { recursive: true })
  writeFileSync(metadata, readFileSync(new URL('../../lib/pages-video-metadata.js', import.meta.url)))
  const result = spawnSync(process.execPath, [script, 'seal', sha], { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(verifyArtifact(sha, join(root, '.build/website')).revision, sha)
}))

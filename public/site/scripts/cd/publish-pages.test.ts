import test from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { CanonicalPropagationPending, differenceMetadata, inventory, main, publicationNeeded, ReadbackTransferIncomplete, seal, SupersededMain, verify, verifyArtifact, verifyReady, type PublisherDeps } from './publish-pages.ts'

const sha = 'a'.repeat(40)
function fixture(root: string) {
  const files: Record<string, string> = {
    'public/site/dist-apparent/index.html': 'source homepage', 'public/site/dist-apparent/404.html': 'not found',
    'public/site/dist-apparent/_routes.json': '{}', 'public/site/dist-apparent/_worker.js': 'compiled worker',
    'public/site/dist-apparent/assets/index-test.js': 'compiled source', 'public/site/dist-apparent/connect/index.html': 'connect',
    'public/site/dist-apparent/connect/device/index.html': 'device', 'public/site/dist-apparent/connect/shares/index.html': 'shares',
    'public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4': 'checked movie',
    'public/site/functions/movie.js': 'source must not reach publisher', 'public/site/wrangler.pages.jsonc': '{"name":"apparent-capture"}',
    'public/site/wrangler.jsonc': 'unrelated Worker', '.env': 'private sentinel',
  }
  for (const [name, data] of Object.entries(files)) { const path = join(root, name); mkdirSync(join(path, '..'), { recursive: true }); writeFileSync(path, data) }
  seal(sha, root)
  return join(root, '.build/website')
}
async function temp<T>(run: (root: string) => Promise<T> | T): Promise<T> {
  const root = mkdtempSync(join(tmpdir(), 'pages-delivery-test-'))
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

test('publication admission checks main ancestry and deployed revision ordering', async () => {
  const calls: string[] = []
  const fetchCompare = async (path: string) => { calls.push(path); return { status: path.includes('...main') ? 'identical' : 'ahead' } }
  assert.equal(await publicationNeeded('example/project', 'c'.repeat(40), 'b'.repeat(40), fetchCompare), true)
  assert.deepEqual(calls, ['repos/example/project/compare/' + 'c'.repeat(40) + '...main', 'repos/example/project/compare/' + 'b'.repeat(40) + '...' + 'c'.repeat(40)])
  assert.equal(await publicationNeeded('example/project', sha, sha, async () => ({ status: 'identical' })), false)
  await assert.rejects(publicationNeeded('bad repo', sha, null), /Invalid repository/)
  await assert.rejects(publicationNeeded('example/project', sha, null, async () => ({ status: 'behind' })), /no longer on main/)
  let divergeCall = 0
  await assert.rejects(publicationNeeded('example/project', sha, 'b'.repeat(40), async () => ({ status: ++divergeCall === 1 ? 'identical' : 'diverged' })), /diverge/)
})

test('sealed artifact rejects revision changes, content edits, extra inputs, symlinks and reuse', () => temp(root => {
  const artifact = fixture(root)
  assert.equal(verifyArtifact(sha, artifact).revision, sha)
  assert.throws(() => verifyArtifact('b'.repeat(40), artifact), /revision/)
  const file = join(artifact, 'dist-apparent/index.html'), original = readFileSync(file)
  writeFileSync(file, 'tampered'); assert.throws(() => verifyArtifact(sha, artifact), /checksum/); writeFileSync(file, original)
  const extra = join(artifact, 'dist-apparent/extra.js'); writeFileSync(extra, 'unexpected'); assert.throws(() => verifyArtifact(sha, artifact), /checksum/); rmSync(extra)
  symlinkSync(root, extra, 'dir'); assert.throws(() => inventory(artifact), /symlink/); rmSync(extra)
  assert.throws(() => seal(sha, root), /EEXIST/)
}))

test('readback binds revision before any public HTTP request', async () => {
  let requested = false
  await assert.rejects(verify('b'.repeat(40), '/unused', { api: async () => project(), fetch: async () => { requested = true; return new Response() } }), /checked revision/)
  assert.equal(requested, false)
})

test('readback verifies HTML, JS, movie, privacy headers and both origins', () => temp(async root => {
  const artifact = fixture(root), packagePath = join(artifact, 'dist-apparent')
  const bodies: Record<string, string> = { '/': 'index.html', '/connect/': 'connect/index.html', '/connect/device/': 'connect/device/index.html', '/connect/shares/': 'connect/shares/index.html', '/assets/index-test.js': 'assets/index-test.js' }
  const visited = new Set<string>()
  let badPath: string | null = null, badOrigin: string | null = null, shortMovie = 0, corruptMovie = false
  const fetcher: typeof fetch = async input => {
    const url = new URL(String(input)); visited.add(url.host)
    const headers = new Headers({ 'X-Apparent-Publisher': 'cloudflare-pages', 'Cache-Control': 'no-store', 'X-Robots-Tag': 'noindex, nofollow', 'Content-Type': 'video/mp4', 'Set-Cookie': 'private-cookie-sentinel' })
    if (url.pathname === badPath && (badOrigin === null || url.host === badOrigin)) return new Response('unexpected response body sentinel', { headers })
    if (url.pathname.endsWith('.mp4')) {
      if (shortMovie > 0) {
        shortMovie--
        const broken = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(Buffer.from('checked mo')); controller.error(new TypeError('premature socket close')) } })
        return new Response(broken, { headers: new Headers({ ...Object.fromEntries(headers), 'Content-Length': '13' }) })
      }
      const movie = corruptMovie ? 'xxxxxxxxxxxxx' : 'checked movie'
      headers.set('Content-Length', String('checked movie'.length)); return new Response(movie, { headers })
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
  for (const missing of ['X-Apparent-Publisher', 'Cache-Control', 'X-Robots-Tag']) {
    const headers = new Headers({ 'X-Apparent-Publisher': 'cloudflare-pages', 'Cache-Control': 'no-store', 'X-Robots-Tag': 'noindex, nofollow', 'Content-Type': 'video/mp4' }); headers.delete(missing)
    await assert.rejects(verify(sha, packagePath, { ...deps, fetch: async () => new Response('connect', { headers }) }))
  }
}))

test('publisher stages exact bytes, writes success receipt, skips superseded candidates, and fails closed', () => temp(async root => {
  const artifact = fixture(root)
  const movieBytes = readFileSync(join(root, 'public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4'))
  rmSync(join(root, 'public/site'), { recursive: true })
  const oldEnv = { sha: process.env.GITHUB_SHA, ref: process.env.GITHUB_REF, token: process.env.WEBSITE_PAGES_API_TOKEN, repo: process.env.GITHUB_REPOSITORY }
  process.env.GITHUB_SHA = sha; process.env.GITHUB_REF = 'refs/heads/main'; process.env.WEBSITE_PAGES_API_TOKEN = 'synthetic'; process.env.GITHUB_REPOSITORY = 'example/project'
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
        const paths: Record<string, string> = { '/': 'index.html', '/connect/': 'connect/index.html', '/connect/device/': 'connect/device/index.html', '/connect/shares/': 'connect/shares/index.html', '/assets/index-test.js': 'assets/index-test.js' }
        return new Response(readFileSync(join(packagePath, paths[url.pathname])), { headers })
      },
    }
    await main(deps)
    const receipt = JSON.parse(readFileSync(join(root, 'website-receipt.json'), 'utf8'))
    assert.equal(receipt.revision, sha)
    assert.equal(receipt.deployment_id, 'deployment')
    assert.equal(staged.length, 1)
    assert.equal(existsSync(staged[0]), false)

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
    for (const [key, value] of Object.entries({ GITHUB_SHA: oldEnv.sha, GITHUB_REF: oldEnv.ref, WEBSITE_PAGES_API_TOKEN: oldEnv.token, GITHUB_REPOSITORY: oldEnv.repo })) {
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

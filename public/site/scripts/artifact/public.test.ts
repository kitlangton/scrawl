import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, dirname } from 'node:path'
import { inventory, resourceGraph, safePath, validatePreservedFiles, validatePublic } from './public.ts'
import { preparePublic } from './prepare.ts'

test('Public sealed artifact builds receive the existing EU ingestion configuration', () => {
  const workflow = readFileSync(new URL('../../../../.github/workflows/stream-public.yml', import.meta.url), 'utf8')
  const build = workflow.split('      - name: Verify Public source, browsers, shared inputs and prepared movie worker\n')[1].split('      - name: Seal')[0]
  assert.match(build, /PUBLIC_POSTHOG_TOKEN: \$\{\{ secrets\.APPARENT_POSTHOG_TOKEN \}\}/)
  assert.match(build, /PUBLIC_POSTHOG_HOST: https:\/\/eu\.i\.posthog\.com/)
  assert.match(build, /PUBLIC_POSTHOG_ENVIRONMENT: production/)
  assert.match(build, /run: npm run verify/)
})

test('Public preparation preserves the raw graph and refuses Connect before writing metadata', () => {
  const f = fixture()
  try {
    const raw = inventory(f.root)
    preparePublic(f.root, new Map())
    validatePreservedFiles(inventory(f.root), raw)
    assert.equal(inventory(f.root).get('_redirects')!.toString(), '')
  } finally { f.cleanup() }
  const contaminated = fixture()
  try {
    contaminated.put('connect/index.html', 'account page')
    assert.throws(() => preparePublic(contaminated.root, new Map()), /Unexpected Public output/)
    assert(!inventory(contaminated.root).has('_headers'))
  } finally { contaminated.cleanup() }
})

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'public-artifact-'))
  const files = new Map<string, Buffer>()
  const put = (path: string, body: string | Buffer) => {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), body); files.set(path, Buffer.isBuffer(body) ? body : Buffer.from(body))
  }
  put('index.html', '<!doctype html><link rel="stylesheet" href="/assets/home-a.css"><script type="module" src="/assets/home-a.js"></script><img srcset="data:image/png;base64,abc 1x, /social-preview-v1.png 2x">')
  put('assets/home-a.js', 'import "./child-a.js"; const preload=["assets/child-a.js"];')
  put('assets/child-a.js', 'export const site = true')
  put('assets/home-a.css', '@import "./child-a.css"; body{background:url(data:image/png;base64,abc)}')
  put('assets/child-a.css', 'body{color:#123}')
  put('social-preview-v1.png', 'image bytes')
  return { root, files, put, cleanup: () => rmSync(root, { recursive: true, force: true }) }
}
function canonical(f: ReturnType<typeof fixture>, path: string, body: string) { f.put(`assets/${path}`, body); f.put(path, body) }

test('validates a Public homepage graph with imports, CSS URLs, srcset, data and fragment references', () => {
  const f = fixture()
  try { assert.doesNotThrow(() => validatePublic(f.files)); assert(resourceGraph(f.files, ['index.html']).has('assets/child-a.css')) }
  finally { f.cleanup() }
})

test('accepts canonical grouped assets only when bytes match and rejects Connect routes', () => {
  const f = fixture()
  try {
    canonical(f, 'pond/frog-air-cohesive.png', 'canonical pond')
    canonical(f, 'team/code-wordmark.svg', 'canonical team')
    canonical(f, 'feature-demos/anywhere/index.html', '<title>demo</title>')
    const canonicalFiles = new Map<string, Buffer>()
    for (const path of ['pond/frog-air-cohesive.png', 'team/code-wordmark.svg', 'feature-demos/anywhere/index.html']) {
      canonicalFiles.set(`assets/${path}`, f.files.get(`assets/${path}`)!)
      f.files.delete(`assets/${path}`)
    }
    assert.doesNotThrow(() => validatePublic(f.files, canonicalFiles))
    f.put('connect/index.html', '<p>Connect</p>')
    assert.throws(() => validatePublic(f.files), /Unexpected Public output/)
  } finally { f.cleanup() }
})

test('rejects missing resources, owner contamination, private content and unreferenced files', () => {
  for (const [path, body, pattern] of [
    ['assets/home-a.js', 'import "./missing.js"', /Missing resource/],
    ['assets/home-a.js', 'void import("./missing-dynamic.js")', /Missing resource/],
    ['assets/home-a.js', 'const header="X-CSRF-Token"', /Wrong resource owner/],
    ['assets/unused-a.js', 'const panel="hosted-panel"', /Wrong resource owner/],
    ['assets/home-a.js', 'const secret="synthetic-csrf"', /Private Public content/],
    ['assets/orphan.js', 'unused', /Unexpected Public output/],
    ['connect/device/index.html', 'not Public', /Unexpected Public output/],
  ] as const) {
    const f = fixture()
    try { f.put(path, body); assert.throws(() => validatePublic(f.files), pattern) }
    finally { f.cleanup() }
  }
})

test('rejects malformed CSS, dynamic missing imports, unsafe paths and symlinks', () => {
  const f = fixture()
  try {
    f.put('assets/home-a.css', 'body{background:URL(/missing.png)}')
    assert.throws(() => validatePublic(f.files), /Missing resource/)
    f.put('assets/home-a.css', 'body{background:url("unterminated)}')
    assert.throws(() => validatePublic(f.files), /Unsupported CSS URL syntax/)
    for (const path of ['../escape', '/root', 'a//b', 'a/./b', 'a\\b', 'a?b', 'a#b', 'a%2fb']) assert.throws(() => safePath(path))
    symlinkSync('/etc/hostname', join(f.root, 'linked'))
    assert.throws(() => inventory(f.root), /regular artifact files/)
  } finally { f.cleanup() }
})

test('prepared metadata admits a clean Public 404 and publisher headers, rejects hosted redirects', () => {
  const f = fixture()
  try {
    f.put('404.html', '<!doctype html><html lang="en"><meta name="robots" content="noindex"><title>Not found</title><body>Not found</body></html>\n')
    f.put('_headers', '/*\n  Cache-Control: no-cache, no-transform\n  X-Apparent-Publisher: cloudflare-pages\n')
    f.put('_redirects', '')
    assert.doesNotThrow(() => validatePublic(f.files, new Map(), { prepared: true }))
    f.put('_redirects', '/hosted /connect 302\n')
    assert.throws(() => validatePublic(f.files, new Map(), { prepared: true }), /Unexpected Public redirect/)
  } finally { f.cleanup() }
})

test('preservation check requires every original Public byte to survive preparation', () => {
  const f = fixture(), prepared = new Map(f.files)
  try {
    validatePreservedFiles(prepared, f.files)
    prepared.set('index.html', Buffer.from('changed'))
    assert.throws(() => validatePreservedFiles(prepared, f.files), /Missing or changed Public input: index.html/)
  } finally { f.cleanup() }
})

test('raw Public includes its verified movie worker and requires every worker import', () => {
  const f = fixture()
  try {
    f.put('_worker.js', 'import "./assets/worker-a.js"')
    assert.throws(() => validatePublic(f.files), /Missing resource/)
    f.put('assets/worker-a.js', 'export const movie = true')
    assert.doesNotThrow(() => validatePublic(f.files))
  } finally { f.cleanup() }
})

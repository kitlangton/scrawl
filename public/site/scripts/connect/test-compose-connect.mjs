import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync, readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join, dirname } from 'node:path'
import { composeArtifacts, inventory, routes, approved, resourceGraph, safePath, validatePreservedFiles, validatePublic } from './compose-connect.mjs'

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'compose-'))
  const pub = join(root, 'public'), con = join(root, 'connect'), canonical = join(root, 'canonical'), out = join(root, 'out')
  const put = (dir, path, bytes) => { mkdirSync(dirname(join(dir, path)), { recursive: true }); writeFileSync(join(dir, path), bytes) }
  put(pub, 'index.html', '<script type="module" src="/assets/public-a.js"></script>')
  put(pub, 'assets/public-a.js', 'export const publicPage = true;')
  for (const path of approved) { put(canonical, canonicalAssets.get(path), `approved ${path}`); put(con, path, `approved ${path}`) }
  for (const route of routes) put(con, `${route}/index.html`, '<meta name="robots" content="noindex, nofollow"><meta name="referrer" content="no-referrer"><meta http-equiv="Cache-Control" content="no-store"><p>Checking your account</p><script type="module" src="/assets/connect-a.js"></script><link rel="modulepreload" href="/assets/preload-a.js"><link rel="stylesheet" href="/assets/connect-a.css">')
  put(con, 'assets/connect-a.js', 'import "./child-a.js"; const preload=["assets/preload-a.js"]; export const origin="https://hosted.tryapparent.com";')
  put(con, 'assets/child-a.js', 'export const child = true;')
  put(con, 'assets/preload-a.js', 'export const preload = true;')
  put(con, 'assets/connect-a.css', '@import "./child-a.css"; a { background: url(/hosted/source-pond-v4.png) }')
  put(con, 'assets/child-a.css', '@font-face { src: url(/HostGrotesk-Variable.ttf) }')
  put(out, 'previous.bin', Buffer.from([0, 42, 255])); put(out, 'nested/sentinel', 'unchanged')
  return { root, pub, con, canonical, out, put, run: options => composeArtifacts(pub, con, canonical, out, options) }
}
const snapshot = root => [...inventory(root)].sort(([a], [b]) => a.localeCompare(b))
// Replace the complete emitted configuration value, not a helper-only input.
const productionOrigin = 'https://hosted.tryapparent.com'
const canonicalAssets = new Map([
  ['hosted/source-pond-v4.png', 'assets/hosted/source-pond-v4.png'],
  ['hosted/agent-pond-v4.png', 'assets/hosted/agent-pond-v4.png'],
  ['hosted/agent-reading-v4.png', 'assets/hosted/agent-reading-v4.png'],
  ['HostGrotesk-Variable.ttf', 'assets/fonts/HostGrotesk-Variable.ttf'],
  ['apparent-wordmark.svg', 'assets/brand/apparent-wordmark.svg'],
])
for (const origin of [
  'https://hosted.tryapparent.com.attacker.example',
  'https://prefixhosted.tryapparent.com',
  'https://hosted.tryapparent.com@attacker.example',
  'https://user@hosted.tryapparent.com',
  'https://attacker.example/path/https://hosted.tryapparent.com',
  'https://attacker.example/?next=https://hosted.tryapparent.com',
  'https://attacker.example/#https://hosted.tryapparent.com',
  `${productionOrigin}/path`, `${productionOrigin}?query`, `${productionOrigin}#fragment`,
  `${productionOrigin}:444`, `${productionOrigin}:443`, `${productionOrigin}:invalid`,
  'http://hosted.tryapparent.com', '//hosted.tryapparent.com',
  `${productionOrigin}/`, `prefix${productionOrigin}`, `${productionOrigin}suffix`,
  `${productionOrigin} `, `${productionOrigin}\\x2eattacker.example`,
]) test(`refuse non-exact emitted API origin ${JSON.stringify(origin)} without destination mutation`, () => {
  const f = fixture()
  try {
    const before = snapshot(f.out)
    const js = readFileSync(join(f.con, 'assets/connect-a.js'), 'utf8')
    f.put(f.con, 'assets/connect-a.js', js.replace(productionOrigin, origin))
    assert.throws(() => f.run(), /expected Connect API origin/)
    assert.deepEqual(snapshot(f.out), before)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
for (const quote of ['"', "'", '`']) test(`accept complete production origin in emitted ${quote} literal`, () => {
  const f = fixture()
  try {
    const js = readFileSync(join(f.con, 'assets/connect-a.js'), 'utf8')
    f.put(f.con, 'assets/connect-a.js', js.replace(`"${productionOrigin}"`, `${quote}${productionOrigin}${quote}`))
    f.run()
    validatePreservedFiles(inventory(f.out), inventory(f.con))
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
for (const [quote, value] of [
  ['"', "https://attacker.example/#'https://hosted.tryapparent.com'"],
  ["'", 'https://attacker.example/#"https://hosted.tryapparent.com"'],
  ['`', "https://attacker.example/#'https://hosted.tryapparent.com'"],
]) test(`refuse mixed-delimiter URL value before any destination mutation: ${quote}`, () => {
  const f = fixture()
  try {
    const before = snapshot(f.out)
    const js = readFileSync(join(f.con, 'assets/connect-a.js'), 'utf8')
    f.put(f.con, 'assets/connect-a.js', js.replace(`"${productionOrigin}"`, quote + value + quote))
    assert.throws(() => f.run(), /expected Connect API origin/)
    assert.deepEqual(snapshot(f.out), before)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
for (const value of [
  `"${productionOrigin}`, `'${productionOrigin}"`, `\`${productionOrigin}\${suffix}\``,
  `"prefix\\"${productionOrigin}\\"suffix"`,
]) test(`refuse incomplete or escaped origin literal ${JSON.stringify(value)}`, () => {
  const f = fixture()
  try {
    const before = snapshot(f.out)
    const js = readFileSync(join(f.con, 'assets/connect-a.js'), 'utf8')
    f.put(f.con, 'assets/connect-a.js', js.replace(`"${productionOrigin}"`, value))
    assert.throws(() => f.run(), /expected Connect API origin/)
    assert.deepEqual(snapshot(f.out), before)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
for (const syntheticOrigin of [
  'http://127.0.0.1:65536', 'http://127.0.0.1:18809/path', 'http://127.0.0.1:18809?query',
  'http://127.0.0.1:18809#fragment', 'http://127.0.0.1:18809@attacker.example',
  'http://127.0.0.1:18809.attacker.example', 'https://127.0.0.1:18809',
]) test(`refuse invalid synthetic label ${syntheticOrigin} before destination mutation`, () => {
  const f = fixture()
  try {
    const before = snapshot(f.out)
    assert.throws(() => f.run({ syntheticOrigin }), /labelled synthetic loopback origin/)
    assert.deepEqual(snapshot(f.out), before)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
for (const origin of [
  'http://127.0.0.1:18809.attacker.example', 'http://127.0.0.1:18809@attacker.example',
  'http://127.0.0.1:18809/path', 'http://127.0.0.1:18809?query', 'http://127.0.0.1:18809#fragment',
  'http://127.0.0.1:188090', 'http://localhost:18809', 'http://[::1]:18809',
]) test(`refuse labelled synthetic URL boundary bypass ${origin}`, () => {
  const f = fixture()
  try {
    const before = snapshot(f.out)
    f.put(f.con, 'assets/child-a.js', `const origin=${JSON.stringify(origin)}`)
    assert.throws(() => f.run({ syntheticOrigin: 'http://127.0.0.1:18809' }), /Synthetic/)
    assert.deepEqual(snapshot(f.out), before)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('final preservation verification rejects missing unreferenced approved art and differing input bytes', () => {
  const f = fixture()
  try {
    f.run()
    const expected = inventory(f.con)
    validatePreservedFiles(inventory(f.out), expected)
    rmSync(join(f.out, 'hosted/agent-reading-v4.png'))
    assert.throws(() => validatePreservedFiles(inventory(f.out), expected), /Missing or changed composed input: hosted\/agent-reading-v4.png/)
    f.put(f.out, 'hosted/agent-reading-v4.png', 'changed')
    assert.throws(() => validatePreservedFiles(inventory(f.out), expected), /Missing or changed composed input/)
    validatePreservedFiles(inventory(f.con), expected)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('follow inline CSS and srcset resources while allowing fragment/data URLs', () => {
  const f = fixture()
  try {
    f.put(f.pub, 'index.html', String.raw`<style>i{filter:URL(#local)}/* URL(/missing-comment) */b:before{content:"URL(/missing-string)"}[aria-label=Shared\ links]{color:red}</style><div style="background:url(data:image/png;base64,abc)"></div><img srcset="data:image/png;base64,abc 1x, /apparent-wordmark.svg 2x">`)
    f.put(f.pub, 'apparent-wordmark.svg', readFileSync(join(f.canonical, canonicalAssets.get('apparent-wordmark.svg'))))
    f.run()
    assert(resourceGraph(inventory(f.out), ['index.html']).has('apparent-wordmark.svg'))
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('compose independently emitted routes, preloads, import tables, CSS and identical approved collisions', () => {
  const f = fixture()
  try {
    f.put(f.pub, 'apparent-wordmark.svg', readFileSync(join(f.canonical, canonicalAssets.get('apparent-wordmark.svg'))))
    f.run()
    for (const route of routes) assert(inventory(f.out).has(`${route}/index.html`))
    assert.equal(inventory(f.out).has('previous.bin'), false)
    assert.deepEqual([...resourceGraph(inventory(f.out), ['connect/index.html'], true)].sort(), ['connect/index.html', 'assets/connect-a.js', 'assets/child-a.js', 'assets/preload-a.js', 'assets/connect-a.css', 'assets/child-a.css', 'HostGrotesk-Variable.ttf', 'hosted/source-pond-v4.png'].sort())
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('accepts only byte-identical public assets relocated under canonical assets/', () => {
  const f = fixture()
  try {
    const path = 'feature-demos/anywhere/index.html'
    const bytes = Buffer.from('<!doctype html><html><title>illustrative demo</title></html>')
    f.put(f.canonical, `assets/${path}`, bytes)
    f.put(f.pub, path, bytes)
    assert.doesNotThrow(() => validatePublic(inventory(f.pub), inventory(f.canonical)))

    f.put(f.pub, path, Buffer.from(`${bytes} altered`))
    assert.throws(() => validatePublic(inventory(f.pub), inventory(f.canonical)), /Unexpected Public output/)
    f.put(f.pub, path, bytes)
    f.put(f.pub, 'feature-demos/operator/index.html', bytes)
    assert.throws(() => validatePublic(inventory(f.pub), inventory(f.canonical)), /Unexpected Public output/)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('validates relocated Public groups and all Connect served URLs against canonical grouped source paths', () => {
  const f = fixture()
  try {
    const grouped = ['pond/frog-air-cohesive.png', 'team/code-wordmark.svg', 'feature-demos/anywhere/index.html']
    for (const path of grouped) {
      const bytes = Buffer.from(`canonical bytes: ${path}`)
      f.put(f.canonical, `assets/${path}`, bytes)
      f.put(f.pub, path, bytes)
    }
    assert.doesNotThrow(() => validatePublic(inventory(f.pub), inventory(f.canonical)))
    f.run()
    validatePreservedFiles(inventory(f.out), inventory(f.con))

    for (const path of grouped) {
      const bytes = readFileSync(join(f.pub, path))
      f.put(f.pub, path, Buffer.concat([bytes, Buffer.from(' changed')]))
      assert.throws(() => validatePublic(inventory(f.pub), inventory(f.canonical)), /Unexpected Public output/)
      f.put(f.pub, path, bytes)
    }

    rmSync(join(f.canonical, canonicalAssets.get(approved[0])))
    const before = snapshot(f.out)
    assert.throws(() => f.run(), /Changed approved asset: hosted\/source-pond-v4\.png/)
    assert.deepEqual(snapshot(f.out), before)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
for (const [name, mutate, error] of [
  ['different approved collision', f => f.put(f.pub, 'apparent-wordmark.svg', 'different'), /collision/],
  ['different emitted collision', f => f.put(f.pub, 'assets/connect-a.js', 'different'), /collision/],
  ['Connect private path', f => f.put(f.con, 'pitch/index.html', 'private'), /Invalid public path/],
  ['Public private path', f => f.put(f.pub, 'captures/evidence.png', 'private'), /Invalid public path/],
  ['unexpected Connect output', f => f.put(f.con, 'account.json', '{}'), /Unexpected Connect output/],
  ['unexpected Public output', f => f.put(f.pub, 'operator.json', '{}'), /Unexpected Public output/],
  ['unreferenced Connect resource', f => f.put(f.con, 'assets/orphan-a.js', 'unused'), /Unreferenced/],
  ['source map', f => f.put(f.con, 'assets/secret.map', '{}'), /Invalid public path/],
  ['synthetic config', f => f.put(f.con, 'assets/child-a.js', 'const origin="http://127.0.0.1:18809"'), /Synthetic/],
  ['private content', f => f.put(f.con, 'assets/child-a.js', 'const secret="synthetic-csrf"'), /Private/],
  ['Connect marketing graph', f => f.put(f.con, 'assets/child-a.js', 'const css="pond-demo"'), /Wrong resource owner/],
  ['Public API graph', f => f.put(f.pub, 'assets/public-a.js', 'const header="X-CSRF-Token"'), /Wrong resource owner/],
  ['missing import', f => f.put(f.con, 'assets/child-a.js', 'import "./missing-a.js"'), /Missing resource/],
  ['missing preload table', f => f.put(f.con, 'assets/child-a.js', 'const deps=["assets/missing-a.js"]'), /Missing resource/],
  ['missing CSS URL', f => f.put(f.con, 'assets/child-a.css', 'a{background:url(/missing.png)}'), /Missing resource/],
  ['uppercase CSS URL', f => f.put(f.con, 'assets/child-a.css', 'a{background:URL(/missing.png)}'), /Missing resource/],
  ['uppercase external CSS URL', f => f.put(f.con, 'assets/child-a.css', 'a{background:URL(https://other.example/asset.png)}'), /External Connect/],
  ['uppercase CSS import', f => f.put(f.con, 'assets/child-a.css', '@IMPORT "/missing.css";'), /Missing resource/],
  ['uppercase HTML rel token', f => f.put(f.pub, 'index.html', '<link rel="STYLESHEET" href="/missing.css">'), /Missing resource/],
  ['escaped CSS identifier', f => f.put(f.con, 'assets/child-a.css', String.raw`a{background:u\72l(/missing.png)}`), /Unsupported CSS/],
  ['escaped CSS URL', f => f.put(f.con, 'assets/child-a.css', String.raw`a{background:url("\2f missing.png")}`), /Unsupported CSS/],
  ['unsupported CSS URL syntax', f => f.put(f.con, 'assets/child-a.css', 'a{background:url(/* comment */ /missing.png)}'), /Unsupported CSS/],
  ['missing inline style URL', f => f.put(f.pub, 'index.html', '<style>@font-face{src:url(/missing.ttf)}</style>'), /Missing resource/],
  ['missing inline style import', f => f.put(f.pub, 'index.html', '<style>@import "/assets/missing-a.css";</style>'), /Missing resource/],
  ['missing style attribute URL', f => f.put(f.pub, 'index.html', '<div style="background:url(/missing.png)"></div>'), /Missing resource/],
  ['missing srcset candidate', f => f.put(f.pub, 'index.html', '<img srcset="data:image/png;base64,abc 1x, /missing.png 2x">'), /Missing resource/],
  ['missing image preload candidate', f => f.put(f.pub, 'index.html', '<link rel="preload" as="image" imagesrcset="/missing.png 2x">'), /Missing resource/],
  ['font service', f => f.put(f.con, 'assets/child-a.css', '@import "https://fonts.bunny.net/css";'), /External Connect|Wrong resource/],
  ['changed approved bytes', f => f.put(f.con, approved[0], 'changed'), /Changed approved/],
  ['malformed path', f => f.put(f.con, 'assets/bad%2f-a.js', 'bad'), /Invalid public path/],
  ['symlink', f => symlinkSync(join(f.con, 'assets/child-a.js'), join(f.con, 'assets/link-a.js')), /regular artifact/],
  ['nonregular FIFO', f => assert.equal(spawnSync('mkfifo', [join(f.con, 'fifo')]).status, 0), /regular artifact/],
]) test(`refuse ${name} before changing any output byte or entry`, () => {
  const f = fixture()
  try { const before = snapshot(f.out); mutate(f); assert.throws(() => f.run(), error); assert.deepEqual(snapshot(f.out), before) }
  finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('labelled synthetic composition writes only its separate destination and still refuses wrong origins', () => {
  const f = fixture()
  try {
    const production = snapshot(f.pub)
    const js = readFileSync(join(f.con, 'assets/connect-a.js'), 'utf8')
    f.put(f.con, 'assets/connect-a.js', js.replace(productionOrigin, 'http://127.0.0.1:18809'))
    f.run({ syntheticOrigin: 'http://127.0.0.1:18809' })
    assert.deepEqual(snapshot(f.pub), production)
    assert.throws(() => f.run({ syntheticOrigin: 'http://127.0.0.1:18810' }), /Synthetic/)
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})
test('reject malformed manifest/resource paths rather than normalizing them into allowed files', () => {
  for (const path of ['../escape', '/root', 'a//b', 'a/./b', 'a\\b', 'a?b', 'a#b', 'a%2fb']) assert.throws(() => safePath(path))
})
test('labelled synthetic prerendered sign-in navigation retains its path and query', () => {
  const f = fixture()
  try {
    const origin = 'http://127.0.0.1:18809'
    const js = readFileSync(join(f.con, 'assets/connect-a.js'), 'utf8')
    f.put(f.con, 'assets/connect-a.js', js.replace(productionOrigin, origin))
    const path = 'connect/device/index.html'
    const html = readFileSync(join(f.con, path), 'utf8')
    f.put(f.con, path, html + `<a href="${origin}/auth/google/start?returnTo=%2Fconnect%2Fdevice%3Fcode%3D">Continue with Google</a>`)
    f.run({ syntheticOrigin: origin })
    validatePreservedFiles(inventory(f.out), inventory(f.con))
    const before = snapshot(f.out)
    for (const invalid of [origin + '.attacker.example', origin + '@attacker.example', 'http://user@127.0.0.1:18809', 'http://127.0.0.1:18810']) {
      f.put(f.con, path, html + `<a href="${invalid}/auth/google/start">Continue</a>`)
      assert.throws(() => f.run({ syntheticOrigin: origin }), /Synthetic/)
      assert.deepEqual(snapshot(f.out), before)
    }
  } finally { rmSync(f.root, { recursive: true, force: true }) }
})

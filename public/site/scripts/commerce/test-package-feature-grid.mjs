import assert from 'node:assert/strict'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'

const packager = fileURLToPath(new URL('./package-feature-grid.mjs', import.meta.url))
const page = (url, demo = '<section class="pond-demo"></section>') => `<!doctype html><html><head><link rel="canonical" href="${url}"></head><body>${demo}</body></html>`

function run(html, check) {
  const root = mkdtempSync(join(tmpdir(), 'feature-grid-package-'))
  const baseline = join(root, 'baseline'), candidate = join(root, 'candidate'), manifest = join(root, 'private.json')
  try {
    mkdirSync(join(baseline, 'assets'), { recursive: true })
    writeFileSync(join(baseline, 'index.html'), html)
    writeFileSync(join(baseline, 'assets', 'existing.js'), 'existing asset')
    mkdirSync(join(root, 'dist-feature-grid'))
    for (const extension of ['js', 'css']) writeFileSync(join(root, 'dist-feature-grid', `feature-grid.${extension}`), `overlay ${extension}`)
    cpSync(fileURLToPath(new URL('../../public/assets/feature-demos', import.meta.url)), join(root, 'public/assets/feature-demos'), { recursive: true })
    const result = spawnSync(process.execPath, [packager, baseline, candidate, manifest], { cwd: root, encoding: 'utf8' })
    check({ result, baseline, candidate, manifest })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

for (const url of [
  'https://app.tryapparent.com.evil.invalid/', 'https://evil-app.tryapparent.com/',
  'https://evil.invalid/app.tryapparent.com', 'https://evil.invalid/?next=app.tryapparent.com',
  'https://app.tryapparent.com@evil.invalid/', 'https://user:pass@app.tryapparent.com/',
  'http://app.tryapparent.com/', 'https://app.tryapparent.com:8443/',
  'https://app.tryapparent.com/other', 'https://app.tryapparent.com/?next=elsewhere',
  'https://app.tryapparent.com/#elsewhere', '/app.tryapparent.com',
  'https://app.tryapparent.com/?', 'https://app.tryapparent.com/#',
]) {
  test(`refuses unrelated canonical ${url} before writing output`, () => {
    const html = page(url)
    run(html, ({ result, baseline, candidate, manifest }) => {
      assert.notEqual(result.status, 0)
      assert.match(result.stderr, /Unexpected or already-overlaid baseline/)
      assert.equal(existsSync(candidate), false)
      assert.equal(existsSync(manifest), false)
      assert.equal(readFileSync(join(baseline, 'index.html'), 'utf8'), html)
    })
  })
}

for (const html of [
  '<html><head><!-- <link rel="canonical" href="https://app.tryapparent.com/"> --></head><body><section class="pond-demo"></section></body></html>',
  '<html><head><script>const fake = \'<link rel="canonical" href="https://app.tryapparent.com/">\'</script></head><body><section class="pond-demo"></section></body></html>',
  '<html><head><template><link rel="canonical" href="https://app.tryapparent.com/"></template></head><body><section class="pond-demo"></section></body></html>',
  page('https://app.tryapparent.com/', '<p>pond-demo</p>'),
  page('https://app.tryapparent.com/', '<section class="not-pond-demo"></section>'),
  page('https://app.tryapparent.com/', '<!-- <section class="pond-demo"></section> -->'),
  page('https://app.tryapparent.com/', '<template><section class="pond-demo"></section></template>'),
  page('https://app.tryapparent.com/', '<script>"<section class=pond-demo></section>"</script>'),
  page('https://app.tryapparent.com/', '<noscript><section class="pond-demo"></section></noscript>'),
  page('https://app.tryapparent.com/').replace('</head>', '<link rel="canonical" href="https://evil.invalid/"></head>'),
]) {
  test(`refuses text/inert/ambiguous identity: ${html}`, () => run(html, ({ result, candidate, manifest }) => {
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /Unexpected or already-overlaid baseline/)
    assert.equal(existsSync(candidate), false)
    assert.equal(existsSync(manifest), false)
  }))
}

function accepted(html) {
  run(html, ({ result, baseline, candidate, manifest }) => {
    assert.equal(result.status, 0, result.stderr)
    assert.equal(readFileSync(join(candidate, 'assets/existing.js'), 'utf8'), 'existing asset')
    assert.equal(readFileSync(join(baseline, 'index.html'), 'utf8'), html)
    const output = readFileSync(join(candidate, 'index.html'), 'utf8')
    assert.match(output, /feature-grid-overlay-[a-f0-9]+\.js/)
    assert.match(output, /feature-grid-overlay-[a-f0-9]+\.css/)
    assert.equal(existsSync(join(candidate, 'feature-demos/anywhere/index.html')), true)
    assert.equal(existsSync(join(candidate, 'private.json')), false)
    assert.ok(JSON.parse(readFileSync(manifest, 'utf8')).after['assets/existing.js'])
  })
}

test('accepts real canonical and demo section; preserves assets and keeps inventory private', () => accepted(page('https://app.tryapparent.com/')))
test('accepts HTML attribute syntax and URL normalization', () => accepted(page('https://APP.TRYAPPARENT.COM:443/').replace('rel="canonical"', "REL='canonical'").replace('class="pond-demo"', "class='wide pond-demo compact'")))

if (process.env.SITE_BASELINE) {
  test('packages the real artifact without changing its homepage', () => accepted(readFileSync(join(process.env.SITE_BASELINE, 'index.html'), 'utf8')))
}

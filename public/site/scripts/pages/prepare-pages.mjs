// Reconcile the accepted public homepage with freshly built Connect routes.
// The snapshot is public source, not a copy of the repository or release tree.
import { readFileSync, readdirSync, lstatSync, mkdirSync, copyFileSync, writeFileSync } from 'node:fs'
import { resolve, join, dirname, relative } from 'node:path'
import { createHash } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { withAppearance } from './system-appearance.mjs'
const site = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const output = join(site, process.env.COMPOSE_SYNTHETIC_ORIGIN ? 'dist/synthetic-composition' : 'dist-apparent')
const snapshot = join(site, 'published-homepage')
const expected = JSON.parse(readFileSync(join(site, 'published-homepage.sha256.json'), 'utf8'))
const hash = bytes => createHash('sha256').update(bytes).digest('hex')
export function publicPath(path) {
  return /^(?:index\.html|robots\.txt|sitemap\.xml|apparent-wordmark\.svg|icon\.svg|HostGrotesk-Variable\.ttf|social-preview-v1\.png|Apparent-ghost-demo-v4-lilac(?:-poster\.jpg|\.mp4)|apparent-narration-(?:mac|iphone)\.mp3|apparent-demo-pi(?:-poster\.jpg|\.mp4))$/.test(path)
    || /^(?:assets|pond|team|feature-demos)\/.+/.test(path)
    || /^connect(?:\/(?:device|shares))?\/index\.html$/.test(path)
    || /^hosted\/(?:source-pond|agent-pond|agent-reading)-v4\.png$/.test(path)
    || /^hosted(?:\/(?:device|shares))?\/index\.html$/.test(path)
}
function files(root) {
  const result = []
  function walk(dir) {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name), stat = lstatSync(path)
      if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) throw Error('Only regular public files are permitted')
      if (stat.isDirectory()) walk(path)
      else result.push(relative(root, path).split('\\').join('/'))
    }
  }
  walk(root); return result.sort()
}
const actual = files(snapshot)
if (JSON.stringify(actual) !== JSON.stringify(Object.keys(expected).sort())) throw Error('Public snapshot inventory changed; review and update its checksum contract')
for (const path of [...actual, ...files(output)]) if (!publicPath(path)) throw Error(`Path outside the existing public boundary: ${path}`)
for (const path of actual) if (hash(readFileSync(join(snapshot, path))) !== expected[path]) throw Error(`Public snapshot checksum mismatch: ${path}`)
for (const path of actual) {
  const target = join(output, path)
  mkdirSync(dirname(target), { recursive: true })
  copyFileSync(join(snapshot, path), target)
}
// Preserve the accepted source snapshot, applying only the reviewed shared theme.
writeFileSync(join(output, 'index.html'), withAppearance(readFileSync(join(snapshot, 'index.html'), 'utf8')))
// A real 404 disables Pages' default SPA fallback on private/unknown paths.
writeFileSync(join(output, '404.html'), '<!doctype html><html lang="en"><meta name="robots" content="noindex"><title>Not found</title><body>Not found</body></html>\n')
// Pages rejects host-specific source URLs. The existing connect shortcut uses
// narrow zone Redirect Rules; see public/site/scripts/cd/connect_redirects.json.
writeFileSync(join(output, '_redirects'), [
  '/hosted/device /connect/device 302', '/hosted/device/ /connect/device 302',
  '/hosted/shares /connect/shares 302', '/hosted/shares/ /connect/shares 302',
  '/hosted /connect 302', '/hosted/ /connect 302', ''].join('\n'))
writeFileSync(join(output, '_headers'), '/*\n  Cache-Control: no-cache\n  X-Apparent-Publisher: cloudflare-pages\n/connect*\n  Cache-Control: no-store\n  X-Robots-Tag: noindex, nofollow\n/hosted*\n  Cache-Control: no-store\n  X-Robots-Tag: noindex, nofollow\n')
console.log(`Pages public package: retained ${actual.length} accepted homepage files; fresh Connect; unknown paths denied`)

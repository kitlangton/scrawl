// Prepare provider metadata without replacing any source-built public byte.
import { readFileSync, readdirSync, lstatSync, writeFileSync } from 'node:fs'
import { resolve, join, relative } from 'node:path'
const site = resolve(import.meta.dirname, '../..')
const output = join(site, process.env.COMPOSE_SYNTHETIC_ORIGIN ? 'dist/synthetic-composition' : 'dist-apparent')
export function publicPath(path: string) {
  return /^(?:index\.html|robots\.txt|sitemap\.xml|apparent-wordmark\.svg|icon\.svg|HostGrotesk-Variable\.ttf|social-preview-v1\.png|Apparent-ghost-demo-v4-lilac(?:-poster\.jpg|\.mp4)|apparent-narration-(?:mac|iphone)\.mp3|apparent-demo-pi(?:-poster\.jpg|\.mp4))$/.test(path)
    || /^(?:assets|pond|team|feature-demos)\/.+/.test(path)
    || /^connect(?:\/(?:device|shares))?\/index\.html$/.test(path)
    || /^hosted\/(?:source-pond|agent-pond|agent-reading)-v4\.png$/.test(path)
    || /^hosted(?:\/(?:device|shares))?\/index\.html$/.test(path)
}
function files(root: string): string[] {
  const result: string[] = []
  function walk(dir: string) {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name), stat = lstatSync(path)
      if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) throw Error('Only regular public files are permitted')
      if (stat.isDirectory()) walk(path)
      else result.push(relative(root, path).split('\\').join('/'))
    }
  }
  if (lstatSync(root).isSymbolicLink()) throw Error('Only regular public directories are permitted')
  walk(root); return result.sort()
}
const actual = files(output)
for (const path of actual) if (!publicPath(path)) throw Error(`Path outside the existing public boundary: ${path}`)
if (!actual.includes('index.html')) throw Error('Missing source-built homepage')
// A real 404 disables Pages' default SPA fallback on private/unknown paths.
writeFileSync(join(output, '404.html'), '<!doctype html><html lang="en"><meta name="robots" content="noindex"><title>Not found</title><body>Not found</body></html>\n')
// Pages rejects host-specific source URLs. The existing connect shortcut uses
// narrow zone Redirect Rules; see public/site/scripts/cd/connect_redirects.json.
writeFileSync(join(output, '_redirects'), [
  '/hosted/device /connect/device 302', '/hosted/device/ /connect/device 302',
  '/hosted/shares /connect/shares 302', '/hosted/shares/ /connect/shares 302',
  '/hosted /connect 302', '/hosted/ /connect 302', ''].join('\n'))
writeFileSync(join(output, '_headers'), '/*\n  Cache-Control: no-cache\n  X-Apparent-Publisher: cloudflare-pages\n/connect*\n  Cache-Control: no-store\n  X-Robots-Tag: noindex, nofollow\n/hosted*\n  Cache-Control: no-store\n  X-Robots-Tag: noindex, nofollow\n')
console.log(`Pages public package: ${actual.length} source-built files; fresh Connect; unknown paths denied`)

// Prepare provider metadata without replacing any source-built public byte.
import { readFileSync, readdirSync, lstatSync, writeFileSync } from 'node:fs'
import { resolve, join, relative } from 'node:path'
const site = resolve(import.meta.dirname, '../..')
const output = join(site, 'dist-apparent')
export function publicPath(path: string) {
  return /^(?:index\.html|_worker\.js|robots\.txt|sitemap\.xml|apparent-wordmark\.svg|icon\.svg|HostGrotesk-Variable\.ttf|social-preview-v1\.png|Apparent-ghost-demo-v4-lilac(?:-poster\.jpg|\.mp4)|apparent-narration-(?:mac|iphone)\.mp3|apparent-demo-pi(?:-poster\.jpg|\.mp4))$/.test(path)
    || /^(?:assets|pond|team|feature-demos)\/.+/.test(path)
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
// Connect routes and redirects are owned by its independent static Worker.
writeFileSync(join(output, '_redirects'), '')
// Preserve sealed bytes through edge transformations, including automatic RUM injection.
writeFileSync(join(output, '_headers'), '/*\n  Cache-Control: no-cache, no-transform\n  X-Apparent-Publisher: cloudflare-pages\n')
console.log(`Pages public package: ${actual.length} Public-only source-built files; unknown paths denied`)

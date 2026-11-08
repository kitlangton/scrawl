// Artifact-only seam. No browser source or backend is imported by Public.
import { lstatSync, readdirSync, readFileSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { join, dirname, posix, resolve } from 'node:path'
import { parse } from 'parse5'
import { fileURLToPath } from 'node:url'

export const routes = ['connect', 'connect/device', 'connect/shares', 'hosted', 'hosted/device', 'hosted/shares']
export const approved = ['hosted/source-pond-v4.png', 'hosted/agent-pond-v4.png', 'hosted/agent-reading-v4.png', 'HostGrotesk-Variable.ttf', 'apparent-wordmark.svg']
// Canonical public source groups static assets under assets/, while each owner
// serves a deliberate subset at stable root-relative URLs.
const canonicalAssetPaths = new Map([
  ['hosted/source-pond-v4.png', 'assets/hosted/source-pond-v4.png'],
  ['hosted/agent-pond-v4.png', 'assets/hosted/agent-pond-v4.png'],
  ['hosted/agent-reading-v4.png', 'assets/hosted/agent-reading-v4.png'],
  ['HostGrotesk-Variable.ttf', 'assets/fonts/HostGrotesk-Variable.ttf'],
  ['apparent-wordmark.svg', 'assets/brand/apparent-wordmark.svg'],
])
function canonicalAsset(canonical, servedPath) {
  return canonical.get(canonicalAssetPaths.get(servedPath) || `assets/${servedPath}`)
}
const privatePath = /(?:^|\/)(?:\.env[^/]*|.*\.map|pitch[^/]*|captures?|contact-sheet\.png|recording\.(?:mp4|mov)|deployment\.json|node_modules|functions|backend)(?:\/|$)/i
const sensitive = /sourceMappingURL|verified@example|synthetic-csrf|must-not-render|BEGIN (?:RSA |OPENSSH )?PRIVATE KEY|client_secret/
export function safePath(path) {
  if (!path || path.startsWith('/') || /[\\%?#\x00-\x20]/.test(path) || path.split('/').some(part => !part || part === '.' || part === '..') || privatePath.test(path)) throw Error(`Invalid public path: ${path}`)
  return path
}
export function inventory(root) {
  const result = new Map()
  function walk(path = '') {
    const full = join(root, path), stat = lstatSync(full)
    if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) throw Error(`Only regular artifact files permitted: ${path}`)
    if (stat.isDirectory()) for (const name of readdirSync(full)) walk(path ? `${path}/${name}` : name)
    else result.set(safePath(path), readFileSync(full))
  }
  walk(); return result
}
export function mergeFiles(left, right) {
  const result = new Map(left)
  for (const [path, bytes] of right) {
    safePath(path)
    if (result.has(path) && !result.get(path).equals(bytes)) throw Error(`Differing artifact collision: ${path}`)
    result.set(path, bytes)
  }
  return result
}
export function validatePreservedFiles(files, expected) {
  for (const [path, bytes] of expected) if (!files.has(path) || !files.get(path).equals(bytes)) throw Error(`Missing or changed composed input: ${path}`)
}
// Include HTML modulepreloads, emitted static/dynamic imports and Vite preload
// tables (quoted asset filenames), CSS imports and URLs. Navigation is not a fetch.
export function resourceGraph(files, starts, connect = false) {
  const seen = new Set(), pending = [...starts]
  function reference(file, ref) {
    if (/^(?:data:|#)/.test(ref)) return
    if (/^(?:https?:)?\/\//.test(ref)) {
      if (connect) throw Error(`External Connect resource: ${ref}`)
      return
    }
    if (/[\\\x00-\x20%]/.test(ref)) throw Error(`Malformed resource: ${ref}`)
    const clean = ref.split(/[?#]/)[0]
    const target = clean.startsWith('/') ? clean.slice(1) : clean.startsWith('assets/') && file.endsWith('.js') ? clean : posix.normalize(posix.join(posix.dirname(file), clean))
    safePath(target)
    if (!files.has(target)) throw Error(`Missing resource: ${file} -> ${target}`)
    pending.push(target)
  }
  function css(file, text) {
    // Bounded emitted-CSS syntax: skip comments/ordinary strings, recognize
    // case-insensitive URL/import identifiers, and refuse escapes or URL forms
    // outside this grammar instead of silently omitting their resource edges.
    const tokens = /\/\*[\s\S]*?\*\/|"(?:\\[\s\S]|[^"\\])*"|'(?:\\[\s\S]|[^'\\])*'|\\[\s\S]|(?<![-\w])url\s*\(|@import\b/gi
    let token
    while ((token = tokens.exec(text))) {
      if (/^["']|^\/\*/.test(token[0])) continue
      const tail = text.slice(token.index)
      if (token[0].startsWith('\\')) {
        // Vite emits escaped spaces in unquoted attribute selectors. They cannot
        // spell a URL/import identifier; all other identifier escapes fail closed.
        if (token[0] !== '\\ ') throw Error(`Unsupported CSS escape: ${file}`)
        continue
      }
      if (/^url/i.test(token[0])) {
        const match = /^url\(\s*(?:"([^"\\]*)"|'([^'\\]*)'|([^()"'\s\\]*))\s*\)/i.exec(tail)
        if (!match) throw Error(`Unsupported CSS URL syntax: ${file}`)
        reference(file, match[1] ?? match[2] ?? match[3])
        tokens.lastIndex = token.index + match[0].length
      } else {
        const match = /^@import\s*(?:"([^"\\]*)"|'([^'\\]*)'|(?=url\())/i.exec(tail)
        if (!match) throw Error(`Unsupported CSS import syntax: ${file}`)
        if (match[1] !== undefined || match[2] !== undefined) reference(file, match[1] ?? match[2])
        tokens.lastIndex = token.index + match[0].length
      }
    }
  }
  function srcset(file, text) {
    // URLs consume non-space bytes (including commas in data URLs). A trailing
    // comma ends a descriptor-free candidate; otherwise skip its descriptors.
    let rest = text
    while ((rest = rest.replace(/^[\s,]+/, ''))) {
      const url = /^\S+/.exec(rest)[0]
      reference(file, url.replace(/,+$/, ''))
      rest = rest.slice(url.length)
      if (!url.endsWith(',')) rest = rest.includes(',') ? rest.slice(rest.indexOf(',') + 1) : ''
    }
  }
  while (pending.length) {
    const file = pending.pop()
    if (seen.has(file)) continue
    if (!files.has(file)) throw Error(`Missing document: ${file}`)
    seen.add(file)
    if (!/\.(?:html|js|css)$/.test(file)) continue
    const text = files.get(file).toString('utf8')
    if (sensitive.test(text)) throw Error(`Private content in artifact: ${file}`)
    if (connect ? /Visual feedback gets lost|pond-hero|pond-demo|pond-team|fonts\.bunny\.net/.test(text) : /X-CSRF-Token|hosted-panel|hosted-page/.test(text)) throw Error(`Wrong resource owner: ${file}`)
    if (file.endsWith('.html')) {
      function visit(node) {
        const attrs = Object.fromEntries((node.attrs || []).map(a => [a.name, a.value]))
        if (attrs.src) reference(file, attrs.src)
        if (attrs.poster) reference(file, attrs.poster)
        if (attrs.style) css(file, attrs.style)
        if (attrs.srcset) srcset(file, attrs.srcset)
        if (attrs.imagesrcset) srcset(file, attrs.imagesrcset)
        if (node.tagName === 'style') css(file, (node.childNodes || []).map(child => child.value || '').join(''))
        if (node.tagName === 'link' && (attrs.rel || '').toLowerCase().split(/\s+/).some(rel => ['stylesheet', 'modulepreload', 'preload', 'icon'].includes(rel)) && attrs.href) reference(file, attrs.href)
        for (const child of node.childNodes || []) visit(child)
      }
      visit(parse(text))
    } else if (file.endsWith('.css')) {
      css(file, text)
    } else {
      for (const m of text.matchAll(/(?:\bfrom\s*|\bimport\s*(?:\(\s*)?)["']([^"']+)["']|["']((?:\.?\.?\/|\/)?assets\/[^"'\s]+\.(?:js|css)|\.\/[^"'\s]+\.(?:js|css))["']/g)) reference(file, m[1] || m[2])
    }
  }
  return seen
}
export function validateConnect(files, canonical, { syntheticOrigin } = {}) {
  const expectedOrigin = syntheticOrigin || 'https://hosted.tryapparent.com'
  // Origins are complete, canonical URL values. URL.origin alone would discard
  // credentials, paths, queries and fragments, and normalize malformed spellings.
  function exactOrigin(value) {
    try {
      const url = new URL(value)
      return value === url.origin && !url.username && !url.password && url.pathname === '/' && !url.search && !url.hash ? url : null
    } catch { return null }
  }
  if (syntheticOrigin) {
    const url = exactOrigin(syntheticOrigin)
    if (!url || url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || !url.port) throw Error('Expected a labelled synthetic loopback origin')
  }
  const docs = routes.map(route => `${route}/index.html`)
  for (const [path, bytes] of files) {
    if (!(docs.includes(path) || approved.includes(path) || /^assets\/[^/]+-[\w-]+\.(?:js|css)$/.test(path))) throw Error(`Unexpected Connect output: ${path}`)
    if (/\.(?:html|js|css)$/.test(path)) {
      const text = bytes.toString('utf8')
      if (sensitive.test(text)) throw Error(`Private Connect content: ${path}`)
      // Match complete URLs, not the API validator's bare loopback host names.
      // Prerendered sign-in links legitimately add a path/query to the labelled
      // origin; JS configuration remains an exact origin with no URL suffix.
      for (const [value] of text.matchAll(/https?:\/\/[^\s"'`<>]*/gi)) {
        let url
        try { url = new URL(value) } catch { /* A malformed local spelling still refuses below. */ }
        const local = url && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
        if (!local && !/^https?:\/\/(?:127\.0\.0\.1|localhost|\[::1\])/i.test(value)) continue
        if (!syntheticOrigin || !url || url.origin !== expectedOrigin || url.username || url.password
          || (path.endsWith('.html') ? value !== url.origin && value !== url.href : !exactOrigin(value))) throw Error(`Synthetic Connect configuration: ${path}`)
      }
    }
  }
  const js = [...files].filter(([p]) => p.endsWith('.js')).map(([, b]) => b.toString()).join('\n')
  // Vite emits the API configuration as a quoted URL (currently a backtick
  // default argument). Require both value boundaries, not a hostname substring.
  // This bounded artifact grammar accepts only unescaped literal URLs; it does
  // not tokenize unrelated JavaScript, evaluate expressions or decode escapes.
  // Consume the matching delimiter even if another quote kind occurs inside the
  // URL, so an inner quoted hostname cannot be rediscovered as a separate value.
  const literals = js.matchAll(/(?<!\\)(?:"https?:\/\/(?:\\[\s\S]|[^"\\])*"|'https?:\/\/(?:\\[\s\S]|[^'\\])*'|`https?:\/\/(?:\\[\s\S]|[^`\\])*`)/g)
  if (![...literals].some(([literal]) => {
    const value = literal.slice(1, -1)
    const url = exactOrigin(value)
    return url && url.origin === expectedOrigin
  })) throw Error('Missing expected Connect API origin')
  for (const path of docs) {
    const html = files.get(path)?.toString() || ''
    if (!/Checking your account/.test(html) || !/name="robots" content="noindex, nofollow"/.test(html) || !/http-equiv="Cache-Control" content="no-store"/.test(html) || !/name="referrer" content="no-referrer"/.test(html) || /csrfToken|accessToken|application\/ld\+json|rel="canonical"/.test(html)) throw Error(`Invalid Connect loading document: ${path}`)
  }
  for (const path of approved) {
    const bytes = canonicalAsset(canonical, path)
    if (!files.has(path) || !bytes || !files.get(path).equals(bytes)) throw Error(`Changed approved asset: ${path}`)
  }
  const graph = resourceGraph(files, docs, true)
  for (const path of approved) graph.add(path)
  if (graph.size !== files.size) throw Error('Unreferenced Connect output')
}
export function writeArtifact(out, files, inputs = []) {
  for (const input of inputs) if (resolve(out) === resolve(input) || resolve(input).startsWith(resolve(out) + '/') || resolve(out).startsWith(resolve(input) + '/')) throw Error('Output must not overlap an input')
  // Existing output is not an input, but never follow a link while replacing it.
  try { if (lstatSync(out).isSymbolicLink()) throw Error('Output is a symlink') } catch (error) { if (error.code !== 'ENOENT') throw error }
  rmSync(out, { recursive: true, force: true })
  mkdirSync(out, { recursive: true })
  for (const [path, bytes] of files) { mkdirSync(dirname(join(out, path)), { recursive: true }); writeFileSync(join(out, path), bytes) }
}

export function validatePublic(files, canonical) {
  for (const [path, bytes] of files) {
    if (!/^(?:index\.html|robots\.txt|sitemap\.xml|apparent-wordmark\.svg|icon\.svg|HostGrotesk-Variable\.ttf|social-preview-v1\.png|Apparent-ghost-demo-v4-lilac(?:-poster\.jpg|\.mp4))$/.test(path)
      && !/^assets\/[^/]+-[\w-]+\.(?:js|css)$/.test(path)
      && !(/^(?:pond|team|feature-demos)\/.+/.test(path) && canonicalAsset(canonical, path)?.equals(bytes))) throw Error(`Unexpected Public output: ${path}`)
    if (/\.(?:html|js|css|svg)$/.test(path) && sensitive.test(bytes.toString())) throw Error(`Private Public content: ${path}`)
  }
  resourceGraph(files, ['index.html'])
}
export function composeArtifacts(publicRoot, connectRoot, canonicalRoot, out, options = {}) {
  const publicFiles = inventory(publicRoot), connectFiles = inventory(connectRoot), canonical = inventory(canonicalRoot)
  validatePublic(publicFiles, canonical)
  validateConnect(connectFiles, canonical, options)
  const composed = mergeFiles(publicFiles, connectFiles)
  // Both complete inventories, resource graphs, approved bytes and collisions
  // have been checked before replacing any file in the destination.
  writeArtifact(out, composed, [connectRoot, canonicalRoot])
  return composed
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const site = resolve(import.meta.dirname, '../..'), syntheticOrigin = process.env.COMPOSE_SYNTHETIC_ORIGIN
  composeArtifacts(process.env.COMPOSE_PUBLIC_INPUT || join(site, 'dist-apparent'), join(site, '../../connect/web', syntheticOrigin ? 'dist-test' : 'dist'), join(site, 'public'), join(site, syntheticOrigin ? 'dist/synthetic-composition' : 'dist-apparent'), { syntheticOrigin })
  console.log(`Composed independent Public + Connect artifacts (${syntheticOrigin ? 'synthetic only' : 'production origin'})`)
}

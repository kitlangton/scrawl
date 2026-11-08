// Public-only static artifact boundary. This module deliberately has no Connect imports.
import { lstatSync, readdirSync, readFileSync } from 'node:fs'
import { join, posix } from 'node:path'
import { parse } from 'parse5'

const privatePath = /(?:^|\/)(?:\.env[^/]*|.*\.map|pitch[^/]*|captures?|contact-sheet\.png|recording\.(?:mp4|mov)|deployment\.json|node_modules|functions|backend)(?:\/|$)/i
const sensitive = /sourceMappingURL|verified@example|synthetic-csrf|must-not-render|BEGIN (?:RSA |OPENSSH )?PRIVATE KEY|client_secret/
const canonicalFiles = new Set(['index.html','_worker.js','robots.txt','sitemap.xml','404.html','_headers','_redirects','apparent-wordmark.svg','icon.svg','HostGrotesk-Variable.ttf','social-preview-v1.png','Apparent-ghost-demo-v4-lilac.mp4','Apparent-ghost-demo-v4-lilac-poster.jpg'])
const groupedPublic = /^(?:pond|team|feature-demos)\/.+/

export type FileInventory = Map<string, Buffer>

export function safePath(path: string): string {
  if (!path || path.startsWith('/') || /[\\%?#\x00-\x20]/.test(path) || path.split('/').some(part => !part || part === '.' || part === '..') || privatePath.test(path)) throw Error(`Invalid public path: ${path}`)
  return path
}

export function inventory(root: string): FileInventory {
  const result: FileInventory = new Map()
  function walk(path = ''): void {
    const full = join(root, path), stat = lstatSync(full)
    if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) throw Error(`Only regular artifact files permitted: ${path}`)
    if (stat.isDirectory()) for (const name of readdirSync(full)) walk(path ? `${path}/${name}` : name)
    else result.set(safePath(path), readFileSync(full))
  }
  walk(); return result
}

export function validatePreservedFiles(files: FileInventory, expected: FileInventory): void {
  for (const [path, bytes] of expected) if (!files.has(path) || !files.get(path)!.equals(bytes)) throw Error(`Missing or changed Public input: ${path}`)
}

// Follow HTML fetches, emitted JS imports/preload tables and CSS URLs/imports.
// External resources are not artifact edges; malformed local references fail closed.
export function resourceGraph(files: FileInventory, starts: string[]): Set<string> {
  const seen = new Set<string>(), pending = [...starts]
  function reference(file: string, ref: string): void {
    if (/^(?:data:|#)/.test(ref) || /^(?:https?:)?\/\//.test(ref)) return
    if (/[\\\x00-\x20%]/.test(ref)) throw Error(`Malformed resource: ${ref}`)
    const clean = ref.split(/[?#]/)[0]
    const target = clean.startsWith('/') ? clean.slice(1) : clean.startsWith('assets/') && file.endsWith('.js') ? clean : posix.normalize(posix.join(posix.dirname(file), clean))
    safePath(target)
    if (!files.has(target)) throw Error(`Missing resource: ${file} -> ${target}`)
    pending.push(target)
  }
  function css(file: string, text: string): void {
    const tokens = /\/\*[\s\S]*?\*\/|"(?:\\[\s\S]|[^"\\])*"|'(?:\\[\s\S]|[^'\\])*'|\\[\s\S]|(?<![-\w])url\s*\(|@import\b/gi
    let token: RegExpExecArray | null
    while ((token = tokens.exec(text))) {
      if (/^["']|^\/\*/.test(token[0])) continue
      const tail = text.slice(token.index)
      if (token[0].startsWith('\\')) { if (token[0] !== '\\ ') throw Error(`Unsupported CSS escape: ${file}`); continue }
      if (/^url/i.test(token[0])) {
        const match = /^url\(\s*(?:"([^"\\]*)"|'([^'\\]*)'|([^()"'\s\\]*))\s*\)/i.exec(tail)
        if (!match) throw Error(`Unsupported CSS URL syntax: ${file}`)
        reference(file, match[1] ?? match[2] ?? match[3]); tokens.lastIndex = token.index + match[0].length
      } else {
        const match = /^@import\s*(?:"([^"\\]*)"|'([^'\\]*)'|(?=url\())/i.exec(tail)
        if (!match) throw Error(`Unsupported CSS import syntax: ${file}`)
        if (match[1] !== undefined || match[2] !== undefined) reference(file, match[1] ?? match[2]); tokens.lastIndex = token.index + match[0].length
      }
    }
  }
  function srcset(file: string, text: string): void {
    let rest = text
    while ((rest = rest.replace(/^[\s,]+/, ''))) {
      const url = /^\S+/.exec(rest)![0]; reference(file, url.replace(/,+$/, '')); rest = rest.slice(url.length)
      if (!url.endsWith(',')) rest = rest.includes(',') ? rest.slice(rest.indexOf(',') + 1) : ''
    }
  }
  while (pending.length) {
    const file = pending.pop()!
    if (seen.has(file)) continue
    if (!files.has(file)) throw Error(`Missing document: ${file}`)
    seen.add(file)
    if (!/\.(?:html|js|css)$/.test(file)) continue
    const text = files.get(file)!.toString('utf8')
    if (sensitive.test(text)) throw Error(`Private content in Public artifact: ${file}`)
    if (/X-CSRF-Token|hosted-panel|hosted-page|Checking your account/.test(text)) throw Error(`Wrong resource owner: ${file}`)
    if (file.endsWith('.html')) {
      function visit(node: any): void {
        const attrs = Object.fromEntries((node.attrs || []).map((a: any) => [a.name, a.value])) as Record<string,string>
        if (attrs.src) reference(file, attrs.src)
        if (attrs.poster) reference(file, attrs.poster)
        if (attrs.style) css(file, attrs.style)
        if (attrs.srcset) srcset(file, attrs.srcset)
        if (attrs.imagesrcset) srcset(file, attrs.imagesrcset)
        if (node.tagName === 'style') css(file, (node.childNodes || []).map((child: any) => child.value || '').join(''))
        if (node.tagName === 'link' && (attrs.rel || '').toLowerCase().split(/\s+/).some((rel: string) => ['stylesheet','modulepreload','preload','icon'].includes(rel)) && attrs.href) reference(file, attrs.href)
        for (const child of node.childNodes || []) visit(child)
      }
      visit(parse(text))
    } else if (file.endsWith('.css')) css(file, text)
    else for (const m of text.matchAll(/(?:\bfrom\s*|\bimport\s*(?:\(\s*)?)["']([^"']+)["']|["']((?:\.?\.?\/|\/)?assets\/[^"'\s]+\.(?:js|css)|\.\/[^"'\s]+\.(?:js|css))["']/g)) reference(file, m[1] || m[2])
  }
  return seen
}

function publicMetadata(files: FileInventory): void {
  if (files.has('404.html')) {
    const html = files.get('404.html')!.toString('utf8')
    if (!/<title>Not found<\/title>/.test(html) || !/name="robots" content="noindex"/.test(html) || /connect|hosted/i.test(html)) throw Error('Invalid Public 404 document')
  }
  if (files.has('_headers')) {
    const headers = files.get('_headers')!.toString('utf8')
    if (headers !== '/*\n  Cache-Control: no-cache, no-transform\n  X-Apparent-Publisher: cloudflare-pages\n') throw Error('Invalid Public headers metadata')
  }
  if (files.has('_redirects') && files.get('_redirects')!.toString('utf8').trim()) throw Error('Unexpected Public redirect metadata')
}

export function validatePublic(files: FileInventory, canonical: FileInventory = new Map(), options: { prepared?: boolean } = {}): void {
  if (options.prepared) for (const required of ['404.html', '_headers', '_redirects']) if (!files.has(required)) throw Error(`Missing Public metadata: ${required}`)
  for (const [path, bytes] of files) {
    const ordinary = canonicalFiles.has(path) || /^assets\/[^/]+-[\w-]+\.(?:js|css)$/.test(path)
    const grouped = groupedPublic.test(path) && canonical.get(`assets/${path}`)?.equals(bytes)
    if ((!ordinary && !grouped) || (!options.prepared && ['404.html','_headers','_redirects'].includes(path))) throw Error(`Unexpected Public output: ${path}`)
    if (/\.(?:html|js|css)$/.test(path) && /X-CSRF-Token|hosted-panel|hosted-page|Checking your account/.test(bytes.toString())) throw Error(`Wrong resource owner: ${path}`)
    if (/\.(?:html|js|css|svg)$/.test(path) && sensitive.test(bytes.toString())) throw Error(`Private Public content: ${path}`)
  }
  publicMetadata(files)
  const starts = files.has('_worker.js') ? ['index.html','_worker.js'] : ['index.html']
  const graph = resourceGraph(files, starts)
  for (const path of files.keys()) if (path.endsWith('.html') && path !== '404.html' && !graph.has(path)) resourceGraph(files, [path])
  for (const path of files.keys()) if (!graph.has(path) && !canonicalFiles.has(path) && !groupedPublic.test(path) && !/^assets\/[^/]+-[\w-]+\.(?:js|css)$/.test(path)) throw Error(`Unreferenced Public output: ${path}`)
}

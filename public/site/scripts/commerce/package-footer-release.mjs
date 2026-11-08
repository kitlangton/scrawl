// Scope publication to the accepted footer; retain all other production bytes.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { createRequire, stripTypeScriptTypes } from 'node:module'
import { fileURLToPath } from 'node:url'
import { footerMarkup } from '../../src/footer/markup.ts'
const require = createRequire(import.meta.url)
const { parse } = require(process.env.ACORN_MODULE || 'acorn')
assert.equal(process.argv.length, 5, 'baseline, new candidate and external manifest required')
const [baseline, candidate, manifestPath] = process.argv.slice(2).map(p => path.resolve(p))
assert(!fs.lstatSync(baseline).isSymbolicLink(), 'Baseline must be a real directory, not a symlink; resolve it before packaging')
assert(!fs.existsSync(candidate), 'Refuse to overwrite a candidate')
assert(!manifestPath.startsWith(candidate + path.sep), 'Keep evidence outside the public package')
const site = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const sha = data => crypto.createHash('sha256').update(data).digest('hex')
function inventory(root) {
  return fs.readdirSync(root, {recursive:true,withFileTypes:true}).filter(f => {
    assert(!f.isSymbolicLink(), 'Unexpected package symlink')
    return f.isFile()
  }).map(f => {
    const absolute = path.join(f.parentPath,f.name), data = fs.readFileSync(absolute)
    return {path:path.relative(root,absolute),bytes:data.length,sha256:sha(data)}
  }).sort((a,b)=>a.path.localeCompare(b.path))
}
const before = inventory(baseline)
const html = fs.readFileSync(path.join(baseline,'index.html'),'utf8')
const scripts = [...html.matchAll(/<script type="module" crossorigin src="([^"]+)"><\/script>/g)]
assert.equal(scripts.length,1,'Exactly one React entry')
const oldUrl = scripts[0][1]
assert.match(oldUrl,/^\/assets\/index-[\w-]+\.js$/)
const source = fs.readFileSync(path.join(baseline,oldUrl.slice(1)),'utf8')
const ast = parse(source,{ecmaVersion:'latest',sourceType:'module'})
const nodes = []
function walk(node) {
  if (!node || typeof node.type !== 'string') return
  nodes.push(node)
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) value.forEach(walk)
    else if (value && typeof value === 'object') walk(value)
  }
}
walk(ast)
const value = n => n?.type === 'Literal' ? n.value : n?.type === 'TemplateLiteral' && n.expressions.length === 0 ? n.quasis[0].value.cooked : undefined
const prop = (n,key) => n?.properties?.find(p => (p.key?.name ?? value(p.key)) === key)?.value
const footers = nodes.filter(n=>n.type==='CallExpression' && value(n.arguments[0])==='footer' && ['foot','frog-footer'].includes(value(prop(n.arguments[1],'className'))))
assert.equal(footers.length,1,'Exactly one footer render node')
const node = footers[0], factory = source.slice(node.callee.start,node.callee.end)
const replacement = `(${factory})("footer",{className:"frog-footer",children:(${factory})("apparent-footer",{dangerouslySetInnerHTML:{__html:${JSON.stringify(footerMarkup)}}})})`
const alreadyCurrent = value(prop(node.arguments[1],'className')) === 'frog-footer' && source.slice(node.start,node.end).includes(JSON.stringify(footerMarkup))
const patched = alreadyCurrent ? source : source.slice(0,node.start)+replacement+source.slice(node.end)
parse(patched,{ecmaVersion:'latest',sourceType:'module'})
const newUrl = `/assets/index-${sha(patched).slice(0,16)}.js`
const behavior = stripTypeScriptTypes(fs.readFileSync(path.join(site,'src/footer/behavior.ts'),'utf8'))
parse(behavior,{ecmaVersion:'latest',sourceType:'module'})
const behaviorUrl = `/assets/footer-${sha(behavior).slice(0,16)}.js`
const css = Buffer.concat([fs.readFileSync(path.join(site,'src/footer/footer.css')), Buffer.from('\n'), fs.readFileSync(path.join(site,'src/cta.css'))])
const cssUrl = `/assets/footer-${sha(css).slice(0,16)}.css`
const footer = '<footer class="frog-footer"><apparent-footer>'+footerMarkup+'</apparent-footer></footer>'
const matches = [...html.matchAll(/<footer\b[^>]*>[\s\S]*?<\/footer>/g)]
assert.equal(matches.length,1,'Exactly one SSR footer')
const oldFooter = matches[0][0]
const resources = /<link rel="stylesheet" href="\/assets\/footer-[a-f0-9]+\.css">|<script type="module" src="\/assets\/footer-[a-f0-9]+\.js"><\/script>/g
// Register the new custom element before the baseline React bundle: its
// bundled older footer must not win the first-definition-only registration.
// Keep the CSS last so the shared CTA style overrides baseline styles.
const behaviorResource = `<script type="module" src="${behaviorUrl}"></script>`
const styleResource = `<link rel="stylesheet" href="${cssUrl}">`
const nextHtml = html.replace(oldFooter,footer).replace(resources,'').replace(scripts[0][0],behaviorResource+scripts[0][0].replace(oldUrl,newUrl)).replace('</head>',styleResource+'</head>')
assert.equal(nextHtml.replace(resources,'').replace(newUrl,oldUrl).replace(footer,oldFooter),html.replace(resources,''),'All non-footer HTML survives byte-for-byte')
assert.deepEqual(inventory(baseline),before,'Baseline changed during preparation')
fs.cpSync(baseline,candidate,{recursive:true,errorOnExist:true,force:false})
fs.writeFileSync(path.join(candidate,'index.html'),nextHtml)
function addImmutable(url,data) {
  const target=path.join(candidate,url.replace(/^\//,''))
  if(fs.existsSync(target)) assert.equal(sha(fs.readFileSync(target)),sha(data),`Immutable conflict: ${url}`)
  else { fs.mkdirSync(path.dirname(target),{recursive:true}); fs.writeFileSync(target,data,{flag:'wx'}) }
}
const artUrl='/pond/footer-theatre.webp'
for (const [url,data] of [[newUrl,patched],[behaviorUrl,behavior],[cssUrl,css],[artUrl,fs.readFileSync(path.join(site,'public/assets',artUrl))]]) addImmutable(url,data)
const after=inventory(candidate)
for(const original of before) if(original.path!=='index.html') assert.deepEqual(after.find(f=>f.path===original.path),original,'Existing asset changed')
fs.writeFileSync(manifestPath,JSON.stringify({baseline,candidate,before,after,urls:[newUrl,behaviorUrl,cssUrl,artUrl]},null,2),{flag:'wx'})
console.log(JSON.stringify({candidate,manifestPath,urls:[newUrl,behaviorUrl,cssUrl,artUrl],preservedAssets:before.length-1},null,2))

// Scoped overlay for the live-only movie release. Never rebuild unrelated live UI.
// ACORN_MODULE points at an existing Acorn installation; no dependency install required.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { createRequire, stripTypeScriptTypes } from 'node:module'
import { fileURLToPath } from 'node:url'
import { beliefMarkup } from '../../src/belief/markup.ts'
const require = createRequire(import.meta.url)
const { parse } = require(process.env.ACORN_MODULE || 'acorn')
const [baseline, candidate, manifestPath] = process.argv.slice(2).map(p => path.resolve(p))
assert(baseline && candidate && manifestPath, 'baseline, new candidate, external manifest required')
assert(!fs.existsSync(candidate), 'Refuse to overwrite a candidate')
assert(!manifestPath.startsWith(candidate + path.sep), 'Evidence must stay outside public package')
const site = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const sha = data => crypto.createHash('sha256').update(data).digest('hex')
function inventory(root) {
  return fs.readdirSync(root, {recursive: true, withFileTypes: true}).filter(f => {
    assert(!f.isSymbolicLink(), 'Unexpected package symlink')
    return f.isFile()
  }).map(f => {
    const absolute = path.join(f.parentPath, f.name), data = fs.readFileSync(absolute)
    return {path: path.relative(root, absolute), bytes: data.length, sha256: sha(data)}
  }).sort((a,b) => a.path.localeCompare(b.path))
}
const before = inventory(baseline)
const html = fs.readFileSync(path.join(baseline, 'index.html'), 'utf8')
const scripts = [...html.matchAll(/<script type="module" crossorigin src="([^"]+)"><\/script>/g)]
assert.equal(scripts.length, 1)
const oldUrl = scripts[0][1]
assert.match(oldUrl, /^\/assets\/index-[\w-]+\.js$/)
const source = fs.readFileSync(path.join(baseline, oldUrl.slice(1)), 'utf8')
const ast = parse(source, {ecmaVersion:'latest', sourceType:'module'})
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
const sections = nodes.filter(n => n.type === 'CallExpression' && value(n.arguments[0]) === 'section' && value(prop(n.arguments[1], 'className')) === 'pond-purpose')
assert.equal(sections.length, 1, 'Exactly one existing belief render node')
const section = sections[0], factory = source.slice(section.callee.start, section.callee.end)
const custom = `(${factory})("apparent-belief",{className:"belief-pond",dangerouslySetInnerHTML:{__html:${JSON.stringify(beliefMarkup)}}})`
const replacement = `(${factory})("section",{className:"pond-purpose",children:${custom}})`
const patched = source.slice(0, section.start) + replacement + source.slice(section.end)
parse(patched, {ecmaVersion:'latest', sourceType:'module'})
const newUrl = `/assets/index-${sha(patched).slice(0,16)}.js`
const behavior = stripTypeScriptTypes(fs.readFileSync(path.join(site,'src/belief/behavior.ts'),'utf8'))
parse(behavior, {ecmaVersion:'latest', sourceType:'module'})
const behaviorUrl = `/assets/belief-${sha(behavior).slice(0,16)}.js`
const css = fs.readFileSync(path.join(site,'src/belief/belief.css'))
const cssUrl = `/assets/belief-${sha(css).slice(0,16)}.css`
const sectionStart = html.indexOf('<section class="pond-purpose">')
assert(sectionStart >= 0)
// The belief itself contains a nested study section after its first publication.
let sectionEnd = -1, depth = 0
for (const tag of html.slice(sectionStart).matchAll(/<\/?section\b[^>]*>/g)) {
  depth += tag[0].startsWith('</') ? -1 : 1
  if (depth === 0) { sectionEnd = sectionStart + tag.index + tag[0].length; break }
}
assert(sectionEnd > sectionStart, 'Balanced belief section required')
assert(!html.slice(sectionEnd).includes('<section class="pond-purpose">'))
const oldResources = /<link rel="stylesheet" href="\/assets\/belief-[a-f0-9]+\.css">|<script type="module" src="\/assets\/belief-[a-f0-9]+\.js"><\/script>/g
const cleanHtml = html.replace(oldResources, '')
let nextHtml = (html.slice(0,sectionStart) + `<section class="pond-purpose"><apparent-belief class="belief-pond">${beliefMarkup}</apparent-belief></section>` + html.slice(sectionEnd)).replace(oldResources, '')
nextHtml = nextHtml.replace(oldUrl,newUrl).replace('</head>',`<link rel="stylesheet" href="${cssUrl}"><script type="module" src="${behaviorUrl}"></script></head>`)
// All page bytes outside the belief, linked root bundle and added resources must survive.
assert.equal(nextHtml.replace(`<link rel="stylesheet" href="${cssUrl}"><script type="module" src="${behaviorUrl}"></script>`,'').replace(newUrl,oldUrl).replace(`<section class="pond-purpose"><apparent-belief class="belief-pond">${beliefMarkup}</apparent-belief></section>`,html.slice(sectionStart,sectionEnd)),cleanHtml)
assert.deepEqual(inventory(baseline),before,'Baseline changed during preparation')
fs.cpSync(baseline,candidate,{recursive:true,errorOnExist:true,force:false})
fs.writeFileSync(path.join(candidate,'index.html'),nextHtml)
function addImmutable(relative, data) {
  const target = path.join(candidate, relative)
  if (fs.existsSync(target)) assert.equal(sha(fs.readFileSync(target)), sha(data), `Immutable asset conflict: ${relative}`)
  else { fs.mkdirSync(path.dirname(target), {recursive:true}); fs.writeFileSync(target,data,{flag:'wx'}) }
}
for (const [url,data] of [[newUrl,patched],[behaviorUrl,behavior],[cssUrl,css]]) addImmutable(url.slice(1),data)
for (const asset of inventory(path.join(site,'public/assets/pond/belief'))) addImmutable(`pond/belief/${asset.path}`,fs.readFileSync(path.join(site,'public/assets/pond/belief',asset.path)))
const after = inventory(candidate)
for (const original of before) if(original.path !== 'index.html') assert.deepEqual(after.find(f=>f.path===original.path),original,'Existing asset changed')
const added = after.filter(f=>!before.some(b=>b.path===f.path)).map(f=>f.path)
assert(added.every(p=>[newUrl,behaviorUrl,cssUrl].some(url=>url.slice(1)===p)||p.startsWith('pond/belief/')))
assert(!nextHtml.includes('Scroll to try the marks') && !nextHtml.includes('/home/simon') && !nextHtml.includes('I agree'))
assert.deepEqual(inventory(baseline),before)
const manifest = {baseline,candidate,before,after,changed:['index.html'],added:after.filter(f=>!before.some(b=>b.path===f.path)).map(f=>f.path),urls:[newUrl,behaviorUrl,cssUrl],scope:'Approved Me too frog belief and direct reversible scroll marks only; every pre-existing asset unchanged.',createdAt:new Date().toISOString()}
fs.writeFileSync(manifestPath,JSON.stringify(manifest,null,2),{flag:'wx',mode:0o600})
console.log(JSON.stringify({candidate,manifestPath,urls:manifest.urls,preservedAssets:before.length-1,added:manifest.added},null,2))

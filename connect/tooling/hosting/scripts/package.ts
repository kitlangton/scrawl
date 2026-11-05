import { lstat, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { join, relative, resolve } from "node:path"

const packageRoot = resolve(import.meta.dirname, "..")
const source = resolve(packageRoot, "../../web/dist")
const output = resolve(packageRoot, ".output/dist")
const browserRoutes = ["connect", "connect/device", "connect/shares"]
const legacyRoutes = ["hosted", "hosted/device", "hosted/shares"]
const publicArt = ["hosted/source-pond-v4.png", "hosted/agent-pond-v4.png", "hosted/agent-reading-v4.png"]
const browserAssets = ["HostGrotesk-Variable.ttf", "apparent-wordmark.svg"]
const routeDocuments = new Set(browserRoutes.map(route => `${route}/index.html`))

async function inventory(root: string): Promise<Map<string, Buffer>> {
  const result = new Map<string, Buffer>()
  async function walk(path = ""): Promise<void> {
    const full = join(root, path)
    const stat = await lstat(full)
    if (stat.isSymbolicLink()) throw new Error(`symlink in browser artifact: ${path}`)
    if (stat.isDirectory()) {
      for (const name of await readdir(full)) await walk(path ? `${path}/${name}` : name)
    } else if (stat.isFile()) result.set(path, await readFile(full))
    else throw new Error(`unsupported browser artifact input: ${path}`)
  }
  await walk()
  return result
}

function target(path: string): string {
  if (path.startsWith("assets/")) return `connect/${path}`
  if (browserAssets.includes(path)) return `connect/assets/${path}`
  if (path.startsWith("hosted/")) return `connect/assets/${path.slice("hosted/".length)}`
  return path
}

const inputs = await inventory(source)
for (const route of browserRoutes) if (!inputs.has(`${route}/index.html`)) throw new Error(`missing route document: ${route}`)
for (const asset of [...publicArt, ...browserAssets]) if (!inputs.has(asset)) throw new Error(`missing approved browser asset: ${asset}`)
for (const path of inputs.keys()) {
  if (legacyRoutes.some(route => path === `${route}/index.html`)) continue
  if (!(routeDocuments.has(path) || publicArt.includes(path) || browserAssets.includes(path) || /^assets\/[^/]+-[\w-]+\.(?:js|css)$/.test(path))) throw new Error(`unexpected browser artifact input: ${path}`)
}

const packaged = new Map<string, Buffer>()
for (const [path, original] of inputs) {
  if (legacyRoutes.some(route => path === `${route}/index.html`)) continue
  let bytes = original
  if (/\.(?:html|js|css)$/.test(path)) {
    let text = original.toString("utf8")
    // This packaging profile preserves Public's accepted browser artifact while
    // rewriting the dedicated Worker graph to paths owned by /connect*.
    text = text.replaceAll("/assets/", "/connect/assets/")
      .replaceAll("/hosted/", "/connect/assets/")
      .replaceAll("/HostGrotesk-Variable.ttf", "/connect/assets/HostGrotesk-Variable.ttf")
      .replaceAll("/apparent-wordmark.svg", "/connect/assets/apparent-wordmark.svg")
    if (/['"`]\/(?:assets|hosted)\//.test(text)) throw new Error(`root browser asset URL remains: ${path}`)
    bytes = Buffer.from(text)
  }
  const served = target(path)
  if (packaged.has(served)) throw new Error(`package path collision: ${served}`)
  packaged.set(served, bytes)
}

for (const route of browserRoutes) {
  const html = packaged.get(`${route}/index.html`)?.toString() ?? ""
  if (!/name="robots" content="noindex, nofollow"/.test(html) || !/name="referrer" content="no-referrer"/.test(html) || !/http-equiv="Cache-Control" content="no-store"/.test(html)) throw new Error(`privacy metadata missing: ${route}`)
  for (const match of html.matchAll(/<(?:script|img|link)\b[^>]*?\b(?:src|href)=["']([^"']+)["']/gi)) {
    const url = new URL(match[1], `https://artifact.invalid/${route}/`)
    if (url.origin !== "https://artifact.invalid") throw new Error(`external route resource: ${route} -> ${match[1]}`)
    const resourcePath = decodeURIComponent(url.pathname.slice(1))
    if (!packaged.has(resourcePath)) throw new Error(`missing packaged route resource: ${route} -> ${resourcePath}`)
  }
}
const visited = new Set<string>()
const pending = browserRoutes.map(route => `${route}/index.html`)
while (pending.length) {
  const path = pending.pop()!
  if (visited.has(path)) continue
  const bytes = packaged.get(path)
  if (!bytes) throw new Error(`missing Worker graph node: ${path}`)
  visited.add(path)
  const text = bytes.toString("utf8")
  const references = path.endsWith(".html")
    ? [...text.matchAll(/<(?:script|img|link)\b[^>]*?\b(?:src|href)=["']([^"']+)["']/gi)].map(match => match[1])
    : path.endsWith(".css")
      ? [...text.matchAll(/url\(["']?([^\s)"']+)/g)].map(match => match[1])
      : path.endsWith(".js")
        ? [...text.matchAll(/["'`](\/(?:connect\/(?:assets|hosted))\/[^"'`]+)["'`]/g)].map(match => match[1])
        : []
  for (const reference of references) {
    if (reference === "/" || /^(?:https?:|data:|#)/.test(reference)) continue
    const url = new URL(reference, `https://artifact.invalid/${path}`)
    if (url.origin !== "https://artifact.invalid") throw new Error(`external Worker graph resource: ${path} -> ${reference}`)
    const targetPath = decodeURIComponent(url.pathname.slice(1))
    if (!packaged.has(targetPath)) throw new Error(`missing Worker graph resource: ${path} -> ${targetPath}`)
    if (/\.(?:html|js|css)$/.test(targetPath)) pending.push(targetPath)
    else visited.add(targetPath)
  }
}
if (visited.size !== packaged.size) throw new Error(`unreferenced Worker output: ${[...packaged.keys()].filter(path => !visited.has(path)).join(", ")}`)
for (const [path, bytes] of packaged) if (/\.(?:html|js|css)$/.test(path) && /https?:\/\/(?:localhost|127\.0\.0\.1)|phc_synthetic|dist-test|verified@example/.test(bytes.toString())) throw new Error(`private or synthetic output: ${path}`)

// Reset only this package-owned generated artifact before writing fresh bytes.
await rm(resolve(packageRoot, ".output"), { recursive: true, force: true })
for (const [path, bytes] of packaged) {
  const full = join(output, path)
  await mkdir(join(full, ".."), { recursive: true })
  await writeFile(full, bytes, { flag: "wx" })
}
console.log(`Packaged static Connect Worker artifact: ${relative(packageRoot, output)} (${packaged.size} files)`)

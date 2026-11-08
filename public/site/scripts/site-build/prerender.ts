// Renders the page to static HTML after `vite build`, so crawlers, link
// previews and agents get the copy without running JavaScript, and the first
// paint doesn't wait for the bundle. The client still mounts with createRoot
// and replaces this markup in one commit.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { audit, MAP_DIR } from "./feature-audit.ts"

process.env.NODE_ENV ??= "production"

const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
const dist = join(site, "dist")
const ssr = join(site, "dist-ssr")

const { render, mapPage } = await import(join(ssr, "entry-server.js"))
let html = readFileSync(join(dist, "index.html"), "utf8")

if (!html.includes("<!--app-->")) throw new Error("prerender: no <!--app--> in dist/index.html")
html = html.replace("<!--app-->", render())

// One small stylesheet: inline it so the first paint needs one request.
html = html.replace(/<link rel="stylesheet"[^>]*href="(\/assets\/[^"]+\.css)"[^>]*>/, (_, href: string) => {
  const css = readFileSync(join(dist, href), "utf8").replaceAll("</style", "<\\/style")
  return `<style>${css}</style>`
})

writeFileSync(join(dist, "index.html"), html)


// The feature map: unlinked, noindex, and left out of the sitemap and llms.txt.
if (process.env.APPARENT_PUBLIC_SITE !== "1") {
  const report = audit()
  if (report.errors.length) throw new Error(`feature map:\n${report.errors.join("\n")}`)
  mkdirSync(join(dist, MAP_DIR), { recursive: true })
  writeFileSync(join(dist, MAP_DIR, "index.html"), mapPage(report))
  console.log(`feature map: /${MAP_DIR}/ (${report.issues.length} to look at)`)
}
rmSync(ssr, { recursive: true, force: true })
console.log(`prerendered dist/index.html (${(html.length / 1024).toFixed(1)} kB)`)

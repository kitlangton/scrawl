// Publish only the Apparent landing page and the assets it actually uses.
// Never copy dist wholesale: it also contains Apparent's update feed, release
// files, agent instructions, and the unlinked feature map.
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { packagePageText } from "./package-page-text.mjs"
import { hasUpstreamLinks } from "../browser-proof/upstream-link-guard.mjs"
import { hasExpectedHomepageCanonical } from "./expected-homepage-canonical.mjs"


const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
const dist = join(site, "dist")
const out = join(site, "dist-apparent")
const html = readFileSync(join(dist, "index.html"), "utf8")
const text = packagePageText(html)
if (hasUpstreamLinks(html)) {
  throw new Error("Refusing to publish upstream download or canonical links")
}
if (!hasExpectedHomepageCanonical(html) || !text.includes("Visual feedback gets lost") || !text.includes("We believe work should be")) {
  throw new Error("Refusing to package a stale or unrelated build")
}
rmSync(out, { recursive: true, force: true })
mkdirSync(out, { recursive: true })
for (const name of [
  "index.html", "assets", "pond", "team", "feature-demos", "apparent-wordmark.svg", "icon.svg", "HostGrotesk-Variable.ttf",
  "social-preview-v1.png",
  "Apparent-ghost-demo-v4-lilac.mp4", "Apparent-ghost-demo-v4-lilac-poster.jpg",
]) {
  if (!existsSync(join(dist, name))) throw new Error(`Missing required asset: ${name}`)
  cpSync(join(dist, name), join(out, name), { recursive: true })
}
writeFileSync(join(out, "robots.txt"), "User-agent: *\nAllow: /\nSitemap: https://app.tryapparent.com/sitemap.xml\n")
writeFileSync(join(out, "sitemap.xml"), '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>https://app.tryapparent.com/</loc></url></urlset>\n')
for (const name of ["appcast.xml", "llms.txt", "skill.md", "og.mp4", "narration.mp3", "narration-iphone.mp3", "releases", "map-tyyxsf5b"]) {
  if (existsSync(join(out, name))) throw new Error(`Unexpected file in public artifact: ${name}`)
}
console.log(`Apparent public artifact: ${out}`)

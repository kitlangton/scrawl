import { existsSync, readdirSync, readFileSync, statSync } from "node:fs"
import { extname, join, relative, sep } from "node:path"

const GROUPS = Object.freeze({
  brand: "",
  "feature-demos": "feature-demos",
  fonts: "",
  hosted: "hosted",
  media: "",
  meta: "",
  pond: "pond",
  releases: "releases",
  team: "team",
})

const CONTENT_TYPES = Object.freeze({
  ".html": "text/html; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".mp3": "audio/mpeg",
  ".mp4": "video/mp4",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".txt": "text/plain; charset=utf-8",
  ".webmanifest": "application/manifest+json",
  ".webp": "image/webp",
  ".xml": "application/xml; charset=utf-8",
  ".ttf": "font/ttf",
  ".ico": "image/x-icon",
})

function filesUnder(directory) {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name)
    return statSync(path).isDirectory() ? filesUnder(path) : [path]
  })
}

export function publicAssets(assetRoot) {
  const entries = []
  for (const [group, servedPrefix] of Object.entries(GROUPS)) {
    const sourceGroup = join(assetRoot, group)
    if (!existsSync(sourceGroup)) continue
    for (const sourcePath of filesUnder(sourceGroup)) {
      const relativePath = relative(sourceGroup, sourcePath).split(sep).join("/")
      const servedPath = [servedPrefix, relativePath].filter(Boolean).join("/")
      entries.push({
        sourcePath,
        servedPath,
        contentType: CONTENT_TYPES[extname(sourcePath).toLowerCase()] ?? "application/octet-stream",
      })
    }
  }
  entries.sort((a, b) => a.servedPath.localeCompare(b.servedPath))
  const seen = new Set()
  for (const entry of entries) {
    if (seen.has(entry.servedPath)) throw new Error(`Duplicate public URL: /${entry.servedPath}`)
    seen.add(entry.servedPath)
  }
  return entries
}

export function readPublicAsset(entry) {
  return readFileSync(entry.sourcePath)
}

export function publicAssetPlugin(assetRoot) {
  const entries = publicAssets(assetRoot)
  const byUrl = new Map(entries.map((entry) => [`/${entry.servedPath}`, entry]))
  return {
    name: "apparent-public-assets",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const urlPath = request.url?.split("?", 1)[0]
        const entry = byUrl.get(urlPath)
        if (!entry) return next()
        response.setHeader("Content-Type", entry.contentType)
        response.end(readPublicAsset(entry))
      })
    },
    generateBundle() {
      if (this.environment?.config.build.ssr) return
      for (const entry of entries) {
        this.emitFile({ type: "asset", fileName: entry.servedPath, source: readPublicAsset(entry) })
      }
    },
  }
}

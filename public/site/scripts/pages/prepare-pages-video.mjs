// Run AFTER package-apparent, BEFORE `wrangler pages deploy dist-apparent`
// from public/site/. Functions remain source code outside the uploaded static tree.
import { closeSync, existsSync, lstatSync, mkdirSync, mkdtempSync, openSync, readSync, renameSync, rmSync, writeFileSync } from "node:fs"
import { createHash } from "node:crypto"
import { join, resolve } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"
import { movie, chunkPath } from "../../lib/pages-video-metadata.js"

function regular(path, directory = false) {
  const stat = lstatSync(path)
  if (stat.isSymbolicLink() || !(directory ? stat.isDirectory() : stat.isFile())) throw new Error(`Not a regular ${directory ? "directory" : "file"}: ${path}`)
  return stat
}
function digest(path, whole = null) {
  regular(path)
  const fd = openSync(path, "r")
  const hash = createHash("sha256")
  const buffer = Buffer.alloc(256 * 1024)
  let size = 0
  try {
    let count
    while ((count = readSync(fd, buffer, 0, buffer.length, null))) {
      hash.update(buffer.subarray(0, count))
      whole?.update(buffer.subarray(0, count))
      size += count
    }
  } finally { closeSync(fd) }
  return { size, sha256: hash.digest("hex") }
}
function verify(path, expected, whole) {
  const actual = digest(path, whole)
  if (actual.size !== expected.size || actual.sha256 !== expected.sha256) throw new Error(`Video identity mismatch: ${path}`)
}

export function preparePagesVideo(output = fileURLToPath(new URL("../../dist-apparent", import.meta.url))) {
  output = resolve(output)
  regular(output, true)
  const source = join(output, movie.path.slice(1))
  const assets = join(output, "assets")
  if (existsSync(assets)) regular(assets, true)
  const hasSource = existsSync(source)
  if (hasSource) verify(source, movie)
  mkdirSync(assets, { recursive: true })
  const staging = mkdtempSync(join(assets, ".pages-video-"))
  try {
    if (hasSource) {
      const fd = openSync(source, "r")
      try {
        for (const chunk of movie.chunks) {
          const buffer = Buffer.alloc(chunk.size)
          let offset = 0
          while (offset < buffer.length) {
            const count = readSync(fd, buffer, offset, buffer.length - offset, null)
            if (!count) throw new Error("Movie changed during preparation")
            offset += count
          }
          const name = chunkPath(chunk).split("/").at(-1)
          const staged = join(staging, name)
          writeFileSync(staged, buffer, { flag: "wx" })
          verify(staged, chunk)
        }
      } finally { closeSync(fd) }
      // All chunks must be fully written and verified before touching source.
      for (const chunk of movie.chunks) {
        const name = chunkPath(chunk).split("/").at(-1)
        const destination = join(assets, name)
        if (existsSync(destination)) verify(destination, chunk)
        else renameSync(join(staging, name), destination)
      }
    }
    const whole = createHash("sha256")
    for (const chunk of movie.chunks) verify(join(output, chunkPath(chunk).slice(1)), chunk, whole)
    if (whole.digest("hex") !== movie.sha256) throw new Error("Reconstructed movie identity mismatch")
    // Recheck source after writing to detect concurrent replacement/modification.
    if (hasSource) verify(source, movie)
    const routes = join(staging, "_routes.json")
    writeFileSync(routes, JSON.stringify({ version: 1, include: [movie.path], exclude: [] }, null, 2) + "\n")
    renameSync(routes, join(output, "_routes.json"))
    if (hasSource) rmSync(source)
  } finally { rmSync(staging, { recursive: true, force: true }) }
  return { size: movie.size, chunks: movie.chunks.length, sha256: movie.sha256 }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length > 2) throw new Error("Usage: node scripts/pages/prepare-pages-video.mjs (run after packaging)")
  console.log("Pages video prepared:", preparePagesVideo())
}

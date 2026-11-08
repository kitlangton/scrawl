// bun run generate:og-video [concept] [--sheet]
// Renders every frame of a loop through headless Chromium and pipes PNGs to ffmpeg
// (H.264 yuv420p, 60 fps, faststart, no audio). Writes public/assets/media/og.mp4, or a contact
// sheet to /tmp with --sheet. Default concept: apparent-pile.
import { chromium } from "playwright-core"
import { spawn } from "node:child_process"
import { statSync } from "node:fs"
import { fileURLToPath } from "node:url"
import path from "node:path"
import { createServer } from "node:http"
import { readFile } from "node:fs/promises"

const types = { ".html": "text/html", ".js": "text/javascript", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2" }

const here = path.dirname(fileURLToPath(import.meta.url))
const args = process.argv.slice(2)
const flag = (n, d) => {
  const i = args.indexOf(n)
  if (i < 0) return d
  const v = args[i + 1]
  args.splice(i, 2)
  return v
}
const crf = flag("--crf", "18")
const sheet = args.includes("--sheet") && args.splice(args.indexOf("--sheet"), 1)
const root = path.resolve(here, "../..")
if (!args.length) args.push("apparent-pile")

const server = createServer(async (req, res) => {
  try {
    const p = path.join(root, decodeURIComponent(new URL(req.url, "http://x").pathname))
    res.writeHead(200, { "content-type": types[path.extname(p)] ?? "application/octet-stream" })
    res.end(await readFile(p))
  } catch {
    res.writeHead(404).end()
  }
}).listen(0)
const port = server.address().port
const browser = await chromium.launch({ args: ["--force-color-profile=srgb", "--disable-lcd-text"] })
for (const name of args) {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 })
  page.on("pageerror", (e) => console.error(name, e))
  await page.goto(`http://127.0.0.1:${port}/scripts/og-video/stage.html?c=${name}&capture`)
  await page.waitForFunction(() => window.ready, null, { timeout: 120000 })
  const frames = await page.evaluate(() => window.frameCount)
  const file = path.join(root, "public/assets/media/og.mp4")
  const crfFor = { "hex-pour": 23, "apparent-pile": 21 }[name] ?? crf
  const vf = sheet ? ["-vf", `select='not(mod(n\\,${Math.ceil(frames / 20)}))',scale=400:-1,tile=5x4`, "-frames:v", "1", `/tmp/${name}-sheet.png`] : null
  const ff = spawn(
    "ffmpeg",
    [
      "-v", "error", "-y", "-f", "image2pipe", "-framerate", "60", "-i", "-",
      ...(vf ?? [
        "-c:v", "libx264", "-preset", "veryslow", "-tune", "animation", "-crf", String(crfFor),
        "-pix_fmt", "yuv420p", "-profile:v", "high", "-level", "4.1",
        "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
        "-movflags", "+faststart", "-an", file,
      ]),
    ],
    { stdio: ["pipe", "inherit", "inherit"] },
  )
  const t0 = Date.now()
  for (let f = 0; f < frames; f++) {
    const url = await page.evaluate((f) => {
      renderFrame(f)
      return document.getElementById("c").toDataURL("image/png")
    }, f)
    const buf = Buffer.from(url.slice(url.indexOf(",") + 1), "base64")
    if (!ff.stdin.write(buf)) await new Promise((r) => ff.stdin.once("drain", r))
  }
  ff.stdin.end()
  await new Promise((r) => ff.on("close", r))
  const size = sheet ? "" : `${(statSync(file).size / 1024).toFixed(0)} KB`
  console.log(`${name}: ${frames} frames in ${((Date.now() - t0) / 1000).toFixed(1)} s ${size}`)
  await page.close()
}
await browser.close()
server.close()

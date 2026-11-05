import { execFileSync } from "node:child_process"
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { chromium } from "playwright-core"

const [beforeArg, afterArg, outputArg, widthArg] = process.argv.slice(2)
if (!beforeArg || !afterArg || !outputArg) {
  throw new Error("Usage: node --experimental-strip-types scripts/motion/clips.ts <before-dir> <after-dir> <out-prefix> [scale-width]")
}
const before = resolve(beforeArg)
const after = resolve(afterArg)
const output = resolve(outputArg)
const width = Number(widthArg ?? 960)
if (!Number.isInteger(width) || width < 2) throw new Error("scale width must be an integer of at least 2")
const temp = mkdtempSync(join(tmpdir(), "apparent-clips-"))
const stripBefore = join(temp, "strip-before.png")
const stripAfter = join(temp, "strip-after.png")
const labels = { Before: join(temp, "Before.png"), After: join(temp, "After.png") }

function run(program: string, args: string[]): void {
  execFileSync(program, args, { stdio: "inherit" })
}

async function writeLabels(): Promise<void> {
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}),
  })
  try {
    const page = await browser.newPage()
    for (const name of ["Before", "After"] as const) {
      const dataUrl = await page.evaluate((label) => {
        const measure = document.createElement("canvas").getContext("2d")!
        measure.font = "bold 22px Arial"
        const width = Math.ceil(measure.measureText(label).width) + 20
        const canvas = document.createElement("canvas")
        canvas.width = width
        canvas.height = 34
        const ctx = canvas.getContext("2d")!
        ctx.fillStyle = "rgba(0,0,0,0.588235)"
        ctx.fillRect(0, 0, width, 34)
        ctx.font = "bold 22px Arial"
        ctx.fillStyle = "white"
        ctx.textBaseline = "alphabetic"
        ctx.fillText(label, 10, 27)
        return canvas.toDataURL("image/png")
      }, name)
      writeFileSync(labels[name], Buffer.from(dataUrl.slice(dataUrl.indexOf(",") + 1), "base64"))
    }
    await page.close()
  } finally {
    await browser.close()
  }
}

function frameCount(directory: string): number {
  return readdirSync(join(directory, "frames")).filter((name) => /^f\d{5}\.png$/.test(name)).length
}

function blankTiles(imagePath: string): number[] {
  const [widthText, heightText] = execFileSync("ffprobe", [
    "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height",
    "-of", "csv=s=x:p=0", imagePath,
  ], { encoding: "utf8" }).trim().split("x")
  const imageWidth = Number(widthText), imageHeight = Number(heightText)
  const pixels = execFileSync("ffmpeg", ["-v", "error", "-i", imagePath, "-f", "rawvideo", "-pix_fmt", "gray", "-"] , { maxBuffer: 128 * 1024 * 1024 })
  const blank: number[] = []
  for (let row = 0; row < 4; row++) {
    const top = Math.floor(row * imageHeight / 4), bottom = Math.floor((row + 1) * imageHeight / 4)
    for (let column = 0; column < 6; column++) {
      const left = Math.floor(column * imageWidth / 6), right = Math.floor((column + 1) * imageWidth / 6)
      let sum = 0, sumSquares = 0, count = 0
      for (let y = top; y < bottom; y++) for (let x = left; x < right; x++) {
        const value = pixels[y * imageWidth + x]
        sum += value
        sumSquares += value * value
        count++
      }
      const variance = count > 1
        ? (sumSquares - sum * sum / count) / (count - 1)
        : 0
      if (Math.sqrt(Math.max(variance, 0)) < 3) blank.push(row * 6 + column)
    }
  }
  return blank
}

try {
  await writeLabels()
  run("ffmpeg", [
    "-v", "error", "-y", "-framerate", "60", "-i", join(before, "frames/f%05d.png"),
    "-framerate", "60", "-i", join(after, "frames/f%05d.png"), "-i", labels.Before, "-i", labels.After,
    "-filter_complex", `[0]scale=${width}:-2[a0];[a0][2]overlay=12:12[a];[1]scale=${width}:-2[b0];[b0][3]overlay=12:12[b];[a][b]hstack=2,pad=ceil(iw/2)*2:ceil(ih/2)*2`,
    "-r", "60", "-c:v", "libx264", "-profile:v", "high", "-pix_fmt", "yuv420p", "-crf", "18", "-movflags", "+faststart", "-an", `${output}.mp4`,
  ])

  const count = frameCount(after)
  if (!count) throw new Error(`No fNNNNN.png frames found in ${join(after, "frames")}`)
  const step = Math.max(1, Math.floor(count / 12))
  for (const [directory, destination] of [[before, stripBefore], [after, stripAfter]] as const) {
    run("ffmpeg", [
      "-v", "error", "-y", "-framerate", "60", "-i", join(directory, "frames/f%05d.png"),
      "-vf", `select='not(mod(n\\,${step}))',scale=${Math.floor(width / 2)}:-2,tile=6x2`,
      "-frames:v", "1", destination,
    ])
  }
  run("ffmpeg", ["-v", "error", "-y", "-i", stripBefore, "-i", stripAfter, "-filter_complex", "vstack=2", `${output}-strip.png`])
  const blank = blankTiles(`${output}-strip.png`)
  console.log(`${output}-strip.png: blank tiles:`, blank.length ? blank : "none")
  console.log(`wrote ${output}.mp4`)
} finally {
  rmSync(temp, { recursive: true, force: true })
}

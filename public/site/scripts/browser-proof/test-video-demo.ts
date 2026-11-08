import assert from "node:assert/strict"
import { readFileSync, existsSync } from "node:fs"
import { createHash } from "node:crypto"
import { test } from "node:test"
import { cameraOffsetAt } from "../../src/demo/core/cursor-camera.ts"
import { DEMO_CURSOR, DEMO_CAPTIONS } from "../../src/demo/content/cursor-track.ts"

test("the approved movie track keeps ordered captions and returns to the ghost result", () => {
  assert(DEMO_CURSOR.every((point, index) => point.t >= 0 && point.x >= 0 && point.x <= 1 && point.y >= 0 && point.y <= 1 && (!index || point.t > DEMO_CURSOR[index - 1].t)))
  assert(DEMO_CAPTIONS.every((cue, index) => cue.text.trim() && cue.start >= 0 && cue.end > cue.start && cue.end <= 59.82 && (!index || cue.start >= DEMO_CAPTIONS[index - 1].end)))
  assert(cameraOffsetAt(49, DEMO_CURSOR) < cameraOffsetAt(35, DEMO_CURSOR) - .2, "the changed ghost is visible after the reload, rather than holding on the agent")
})

test("the phone camera pans continuously without revealing pixels outside the movie", () => {
  const points = [{ t: 0, x: 0, y: .5 }, { t: 4, x: 0, y: .5 }, { t: 6, x: 1, y: .5 }, { t: 10, x: 1, y: .5 }, { t: 12, x: 0, y: .5 }, { t: 16, x: 0, y: .5 }]
  const max = 1 - (4 / 5) / (1916 / 1164)
  assert.equal(cameraOffsetAt(-1, points), 0)
  assert(Math.abs(cameraOffsetAt(8, points) - max) < 1e-12)
  assert.equal(cameraOffsetAt(18, points), 0)
  let previous = 0
  for (let t = 0; t <= 16; t += 1 / 60) {
    const offset = cameraOffsetAt(t, points)
    assert(offset >= 0 && offset <= max)
    assert(Math.abs(offset - previous) < .02, "camera movement remains continuous between decoded frames")
    previous = offset
  }
  assert(Math.abs(cameraOffsetAt(5, points) - cameraOffsetAt(11, points)) < 1e-12, "backward seeks use the same media-clock path")
  const quickMove = [{ t: 0, x: .3, y: .5 }, { t: 4, x: .3, y: .5 }, { t: 5, x: .7, y: .5 }, { t: 9, x: .7, y: .5 }]
  let peakSpeed = 0
  for (let t = 2; t < 7; t += 1 / 60)
    peakSpeed = Math.max(peakSpeed, Math.abs(cameraOffsetAt(t + 1 / 60, quickMove) - cameraOffsetAt(t, quickMove)) * 60)
  assert(peakSpeed < .14, "a rapid cursor jump becomes a slow pan")
})

const artifact = new URL("../../dist-apparent/", import.meta.url)
test("the public demo is the approved lilac movie with the existing playback and chapter controls", () => {
  const html = readFileSync(new URL("index.html", artifact), "utf8")
  assert.equal((html.match(/<video\b/g) ?? []).length, 1, "replace the scripted demo with exactly one real movie")
  assert.match(html, /src="\/Apparent-ghost-demo-v4-lilac\.mp4"/)
  assert.match(html, /poster="\/Apparent-ghost-demo-v4-lilac-poster\.jpg"/)
  assert.match(html, /preload="none"/)
  assert.match(html, /playsInline=""/i)
  for (const label of ["Play with sound", 'aria-label="Play"', 'aria-label="Replay"', 'aria-label="Enable sound"', 'aria-label="Demo timeline"']) assert.ok(html.includes(label), label)
  assert.doesNotMatch(html, /Draw on demo|video-demo-ink/, "the recorded player has no drawing feature")
  for (const label of ["Start", "Capture", "Sketch", "Handoff", "Result", "Try it"]) assert.ok(html.includes(`aria-label="Chapter: ${label}"`), label)
  const movie = readFileSync(new URL("Apparent-ghost-demo-v4-lilac.mp4", artifact))
  assert.equal(createHash("sha256").update(movie).digest("hex"), "89c4621457e573d58a00f99714682eb4bf9351b59de95cabebb944007654d7a7")
  assert.ok(existsSync(new URL("Apparent-ghost-demo-v4-lilac-poster.jpg", artifact)))
})

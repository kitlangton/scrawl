import assert from "node:assert/strict"
import { readFileSync, existsSync } from "node:fs"
import { createHash } from "node:crypto"
import { test } from "node:test"

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

import { expect, test } from "bun:test"
import { latestRelease } from "../src/appcast"
import { latestRelease as mapRelease } from "../scripts/feature-audit"

// Every build is a plain version and build number; a beta build's item carries the channel until
// promotion removes it.
const item = (build: number, version: string, channel = "") => `<item>
  <pubDate>Thu, 22 Oct 2026 12:00:00 -0700</pubDate>
  <sparkle:version>${build}</sparkle:version>
  <sparkle:shortVersionString>${version}</sparkle:shortVersionString>
  ${channel ? `<sparkle:channel>${channel}</sparkle:channel>` : ""}
  <sparkle:minimumSystemVersion>15.0</sparkle:minimumSystemVersion>
</item>`

test("downloads and agent files ignore newer betas regardless of feed order", () => {
  const stable = item(16, "0.3.13")
  const beta = item(18, "0.3.14", "beta")
  for (const xml of [stable + beta, beta + stable]) {
    expect(latestRelease(xml)).toEqual({ version: "0.3.13", build: 16, macos: 15 })
  }
})

test("a promoted build is the newest stable release, ahead of older betas", () => {
  expect(latestRelease(item(19, "0.3.15", "beta") + item(18, "0.3.14") + item(17, "0.3.14", "beta") + item(16, "0.3.13")))
    .toEqual({ version: "0.3.14", build: 18, macos: 15 })
})

test("a beta-only feed does not advertise a beta as stable", () => {
  expect(latestRelease(item(18, "0.3.14", "beta")).build).toBe(0)
})

test("the feature map also reports only stable releases", () => {
  expect(mapRelease(item(18, "0.3.14", "beta") + item(16, "0.3.13")))
    .toEqual({ version: "0.3.13", date: "2026-10-22" })
  expect(mapRelease(item(18, "0.3.14", "beta")))
    .toEqual({ version: "", date: "" })
})

import assert from "node:assert/strict"
import { test } from "node:test"
import { selectLatestDownload } from "../../src/latest-download.ts"

const release = (published_at, names, draft = false) => ({
  published_at, draft,
  assets: names.map((name) => ({ name, browser_download_url: `https://github.com/tryapparent/apparent/releases/download/test/${name}` })),
})

const releases = [
  release("2026-10-29T10:05:08Z", ["Apparent-0.3.13-linux-x86_64.AppImage", "Apparent-0.3.13-linux-aarch64.AppImage", "Apparent-0.3.13-linux-x86_64.AppImage.sha256"]),
  release("2026-10-28T20:58:44Z", ["Apparent-Preview-0.5.1-arm64.dmg", "Apparent-Preview-0.5.1-arm64.zip"]),
  release("2026-10-28T16:54:37Z", ["Apparent-Preview-0.3.5-arm64.dmg"]),
]

test("selects the newest matching release asset, not the newest release overall", () => {
  assert.match(selectLatestDownload(releases, "mac"), /Apparent-Preview-0\.5\.1-arm64\.dmg$/)
  assert.match(selectLatestDownload(releases, "linux-x86_64"), /Apparent-0\.3\.13-linux-x86_64\.AppImage$/)
  assert.match(selectLatestDownload(releases, "linux-aarch64"), /Apparent-0\.3\.13-linux-aarch64\.AppImage$/)
})

test("unsupported Windows and unknown platforms never receive another OS's installer", () => {
  assert.equal(selectLatestDownload(releases, "windows"), null)
  assert.equal(selectLatestDownload(releases, "unsupported"), null)
})

test("does not send a visitor to a draft, checksum, old Scrawl binary, or untrusted asset URL", () => {
  const odd = [
    release("2026-10-31T00:00:00Z", ["Apparent-Preview-0.6.1-arm64.dmg"], true),
    release("2026-10-30T00:00:00Z", ["Scrawl-Preview-0.9.0-arm64.dmg", "Apparent-0.5.0-linux-x86_64.AppImage.sha256"]),
    release("2026-10-29T00:00:00Z", ["Apparent-Preview-0.6.0-arm64.dmg"]),
    ...releases,
  ]
  odd[2].assets[0].browser_download_url = "https://example.com/not-an-installer"
  assert.match(selectLatestDownload(odd, "mac"), /Apparent-Preview-0\.5\.1-arm64\.dmg$/)
})

test("returns null when this platform has no published download", () => {
  assert.equal(selectLatestDownload(releases.filter((r) => !r.assets.some((a) => a.name.endsWith(".dmg"))), "mac"), null)
})

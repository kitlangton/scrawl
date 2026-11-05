import assert from "node:assert/strict"
import { test } from "node:test"
import { desktopOS, linuxArchitecture } from "../../src/platform.ts"

test("only a known desktop OS can receive a platform download", () => {
  assert.equal(desktopOS("Mozilla/5.0 (X11; Linux x86_64) Firefox/143.0", ""), "linux")
  assert.equal(desktopOS("Mozilla/5.0 (X11; Linux aarch64) Firefox/143.0", ""), "linux")
  assert.equal(desktopOS("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", ""), "mac")
  assert.equal(desktopOS("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", ""), "windows")
  assert.equal(desktopOS("Mozilla/5.0 (X11; CrOS x86_64 15572.0.0)", "Linux"), "other")
  assert.equal(desktopOS("Mozilla/5.0 (X11; FreeBSD amd64)", ""), "other")
  assert.equal(desktopOS("Mozilla/5.0 (X11; OpenBSD amd64)", ""), "other")
  assert.equal(desktopOS("Mozilla/5.0 (X11; CrOS aarch64)", "Chrome OS"), "other")
})

test("Linux does not turn a 32-bit or uncertain Client Hint into a 64-bit download", () => {
  const ua = "Mozilla/5.0 (X11; Linux x86_64) Firefox/143.0"
  assert.equal(linuxArchitecture(ua), "x86_64")
  assert.equal(linuxArchitecture(ua, "arm", "64"), "aarch64")
  assert.equal(linuxArchitecture(ua, "x86", "64"), "x86_64")
  assert.equal(linuxArchitecture(ua, "x86", "32"), undefined)
  assert.equal(linuxArchitecture(ua, "arm", "32"), undefined)
  assert.equal(linuxArchitecture(ua, "arm"), undefined)
  assert.equal(linuxArchitecture("Mozilla/5.0 (X11; Linux i686)"), undefined)
})

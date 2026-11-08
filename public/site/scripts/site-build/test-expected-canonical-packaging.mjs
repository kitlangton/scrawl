import assert from "node:assert/strict"
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const site = join(dirname(fileURLToPath(import.meta.url)), "../..")
const homepage = readFileSync(join(site, "dist/index.html"), "utf8")
const canonical = '<link rel="canonical" href="https://app.tryapparent.com/" />'
assert.ok(homepage.includes(canonical), "build the actual public homepage before running these tests")

function packageHomepage(html, accept) {
  const root = mkdtempSync(join(tmpdir(), "apparent-canonical-"))
  try {
    mkdirSync(join(root, "scripts/site-build"), { recursive: true })
    mkdirSync(join(root, "scripts/browser-proof"), { recursive: true })
    for (const [group, names] of [["site-build", ["package-apparent.mjs", "package-page-text.mjs", "expected-homepage-canonical.mjs"]], ["browser-proof", ["upstream-link-guard.mjs"]]]) {
      for (const name of names) cpSync(join(site, "scripts", group, name), join(root, "scripts", group, name))
    }
    symlinkSync(join(site, "node_modules"), join(root, "node_modules"), "dir")
    mkdirSync(join(root, "dist"))
    writeFileSync(join(root, "dist/index.html"), html)
    // Asset contents do not participate in admission. Exercise the real packager
    // with a complete disposable asset inventory, never rewrite the built page.
    for (const name of [
      "connect", "hosted", "assets", "pond", "team", "feature-demos", "apparent-wordmark.svg", "icon.svg", "HostGrotesk-Variable.ttf",
      "social-preview-v1.png", "Apparent-ghost-demo-v4-lilac.mp4", "Apparent-ghost-demo-v4-lilac-poster.jpg",
    ]) writeFileSync(join(root, "dist", name), "fixture asset")
    mkdirSync(join(root, "dist-apparent"))
    writeFileSync(join(root, "dist-apparent/previous-release"), "preserve on rejection")
    const result = spawnSync(process.execPath, [join(root, "scripts/site-build/package-apparent.mjs")], { encoding: "utf8" })
    assert.ifError(result.error)
    if (accept) {
      assert.equal(result.status, 0, result.stderr)
      assert.equal(readFileSync(join(root, "dist-apparent/index.html"), "utf8"), html)
      assert.equal(existsSync(join(root, "dist-apparent/previous-release")), false)
    } else {
      assert.notEqual(result.status, 0, "unsafe canonical was packaged")
      assert.match(result.stderr, /Refusing to package a stale or unrelated build/)
      assert.equal(readFileSync(join(root, "dist-apparent/previous-release"), "utf8"), "preserve on rejection")
      assert.equal(existsSync(join(root, "dist-apparent/index.html")), false)
    }
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

for (const [name, link] of [
  ["actual built homepage", canonical],
  ["HTML case, unquoted attributes and rel tokens", '<LINK HREF=https://APP.tryapparent.com:443 REL="alternate CANONICAL">'],
  ["HTML entities and attribute ordering", "<link href='https://app&#46;tryapparent.com/' title='a > b' rel='canonical'>"],
]) test(`packages ${name}`, () => packageHomepage(homepage.replace(canonical, link), true))

for (const href of [
  "https://evil.example/https://app.tryapparent.com/",
  "https://evil.example/?next=https://app.tryapparent.com/",
  "https://app.tryapparent.com.evil.example/",
  "https://evil-app.tryapparent.com/",
  "https://app.tryapparent.com@evil.example/",
  "https://user:password@app.tryapparent.com/",
  "https://app.tryapparent.com:444/",
  "http://app.tryapparent.com/",
  "https://app.tryapparent.com/wrong",
  "https://app.tryapparent.com/?next=home",
  "https://app.tryapparent.com/#home",
  "https://app.tryapparent.com/?",
  "https://app.tryapparent.com/#",
  "//app.tryapparent.com/",
  "/",
  "not a URL",
]) test(`rejects canonical URL ${href}`, () => {
  packageHomepage(homepage.replace(canonical, `<link rel="canonical" href="${href}">`), false)
})

for (const [name, link] of [
  ["missing canonical despite real Open Graph URL", ""],
  ["comment decoy", `<!-- ${canonical} -->`],
  ["script decoy", `<script type="application/json">${JSON.stringify(canonical)}</script>`],
  ["style decoy", `<style>/* ${canonical} */</style>`],
  ["template decoy", `<template>${canonical}</template>`],
  ["noscript decoy with scripting enabled", `<noscript>${canonical}</noscript>`],
  ["unrelated attribute", '<meta data-canonical="https://app.tryapparent.com/">'],
  ["anchor instead of canonical link", '<a rel="canonical" href="https://app.tryapparent.com/">home</a>'],
  ["missing href", '<link rel="canonical">'],
  ["wrong rel token", '<link rel="notcanonical" href="https://app.tryapparent.com/">'],
  ["duplicate canonical", canonical + canonical],
  ["conflicting canonical", canonical + '<link rel="canonical" href="https://evil.example/">'],
]) test(`rejects ${name}`, () => packageHomepage(homepage.replace(canonical, link), false))

test("rejects canonical in body instead of head", () => {
  packageHomepage(homepage.replace(canonical, "").replace("<body>", `<body>${canonical}`), false)
})

for (const phrase of ["Visual feedback gets lost", "We believe"]) {
  test(`retains existing stale-copy gate for ${phrase}`, () => {
    packageHomepage(homepage.replaceAll(phrase, "outdated copy"), false)
  })
}

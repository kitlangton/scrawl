import assert from "node:assert/strict"
import { cpSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const site = fileURLToPath(new URL("../../", import.meta.url))
const rejected = [
  '<link rel="canonical" href="https://apparent.kitlangton.com/">',
  '<a href="HTTP://APPARENT.KITLANGTON.COM/download">download</a>',
  '<a href="//apparent.kitlangton.com">download</a>',
  '<a href="https://apparent.kitlangton.com.:443">download</a>',
  '<a href=https://apparent.kitlangton.com>download</a>',
  '<a href="https://apparent&#46;kitlangton.com">download</a>',
  '<base href="https://apparent.kitlangton.com/"><a href="release.dmg">download</a>',
  '<a href="https://github.com/kitlangton/tap">tap</a>',
  '<a href="https://github.com/KITLANGTON/homebrew-tap/blob/main/Casks/apparent.rb">tap</a>',
  '<a href="https://github.com/%6bitlangton/%74ap.git">tap</a>',
  '<meta property="og:url" content="https://apparent.kitlangton.com">',
  '<code>brew install --cask kitlangton/tap/apparent</code>',
  '<code>brew install --cask kitlangton&#47;tap/apparent</code>',
  '<meta http-equiv="refresh" content="0;url=https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content="0..5;https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content="0.5.5;https://apparent.kitlangton.com/">',
  '<META HTTP-EQUIV="ReFrEsH" CONTENT=" 0.5 , UrL = &quot;HTTP://APPARENT&#46;KITLANGTON.COM/&quot; ignored">',
  "<meta http-equiv='refr&#101;sh' content='0;&#9;URL&#9;=&#10;&#39;//apparent.kitlangton.com/&#39;'>",
  '<meta http-equiv=refresh content="0 https://apparent.kitlangton.com/">',
  '<meta http-equiv=refresh content=".5;\'https://apparent.kitlangton.com/">',
  { head: '<meta http-equiv=refresh content="0;//apparent.kitlangton.com/"><base href="ftp://example.org/">' },
  '<table><tr><td><base href="ftp://example.org/"></td></tr><div><base href="https://github.com/"></div><meta http-equiv="refresh" content="0;kitlangton/tap"></table>',
  '<table><tr><td><base href="ftp://example.org/"></td></tr><div><template><base href="ftp://inert.example/"></template><base href="https://github&#46;com/"></div><meta http-equiv="refresh" content="0;kitlangton&#47;tap"></table>',
  { head: '<META HTTP-EQUIV=refresh CONTENT="0;URL=&#47;&#47;apparent.kitlangton.com/"><base href="ftp://example.org/">' },
  { head: '<template><base href="ftp://example.org/"></template><meta http-equiv=refresh content="0;//apparent.kitlangton.com/"><base href="ftp://example.org/">' },
  { head: '<template><base href=https://example.org/></template><base href=https://github.com/>', body: '<a href=kitlangton/tap>tap</a>' },
  { head: '<TEMPLATE><template><BASE HREF="https://example.org/"></template></TEMPLATE><base href="https://github&#46;com/">', body: '<a href="kitlangton&#47;tap">tap</a>' },
  { head: '<template><base href="https://example.org/"></template><base href="https://github.com/">', body: '<meta http-equiv="refresh" content="0; URL=kitlangton/tap">' },
  { head: '<base target="_blank"><base href="https://github.com/">', body: '<template><a href="kitlangton/tap">future link</a></template>' },
  { head: '<base href="javascript:void(0)">', body: '<a href="//apparent.kitlangton.com/">download</a>' },
  { head: '<base href="vbscript:legacy">', body: '<a href="//apparent.kitlangton.com/">download</a>' },
  { head: '<base href="data:text/plain,inert">', body: '<meta http-equiv="refresh" content="0;//apparent.kitlangton.com/">' },
  '<template><a href="https://apparent.kitlangton.com/">future link</a></template>',
  '<code>brew install --cask kitlangton/tap/apparent;</code>',
  '<code>echo $(brew install --cask kitlangton/tap/apparent)</code>',
  '<code>(kitlangton/tap/apparent)</code>',
  '<code>brew install kitlangton/tap/apparent&amp;&amp;echo installed</code>',
  '<code>brew install kitlangton/tap/apparent|cat</code>',
  '<code>brew install kitlangton/tap/apparent&gt;log</code>',
]
const accepted = [
  '<a href="https://example.com/?next=https://apparent.kitlangton.com/">other host</a>',
  '<a href="https://example.com/kitlangton/tap">other repo host</a>',
  '<a href="https://apparent.kitlangton.com.example.org/">lookalike</a>',
  '<a href="https://apparent.kitlangton.com@example.org/">userinfo</a>',
  '<a href="https://github.com/other/kitlangton/tap">other owner</a>',
  '<a href="https://github.com/kitlangton/tapestry">other repo</a>',
  '<!-- https://apparent.kitlangton.com/ kitlangton/tap -->',
  '<script>const oldSite = "https://apparent.kitlangton.com/"</script>',
  '<p>Historical site: https://apparent.kitlangton.com/</p>',
  '<code>brew install otherkitlangton/tap/apparent</code>',
  '<meta http-equiv="refresh" content="0;url=https://example.org/?next=https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content="0;url=https://apparent.kitlangton.com.example.org/">',
  '<meta http-equiv="refresh" content="not-a-delay;url=https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content=".;https://apparent.kitlangton.com/">',
  { head: '<meta http-equiv=refresh content="0;kitlangton/tap"><base href="https://github.com/">' },
  { head: '<base href="ftp://example.org/"><meta http-equiv=refresh content="0;//apparent.kitlangton.com/">' },
  '<table><tr><td><base href="https://github.com/"></td></tr><div><base href="ftp://example.org/"></div><meta http-equiv="refresh" content="0;kitlangton/tap"></table>',
  '<table><tr><td><base href="ftp://example.org/"><meta http-equiv="refresh" content="0;kitlangton/tap"></td></tr><div><base href="https://github.com/"></div></table>',
  '<meta http-equiv="refresh" content="0oops;url=https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content="0;url=\'https://example.org/\' https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content="0">',
  '<meta name="description" content="0;url=https://apparent.kitlangton.com/">',
  { head: '<template><base href="https://github.com/"></template><base href="https://example.org/">', body: '<a href="kitlangton/tap">other host</a>' },
  { head: '<base href="https://example.org/"><base href="https://github.com/">', body: '<a href="kitlangton/tap">first active base wins</a>' },
  { head: '<template><base href="https://github.com/"></template>', body: '<a href="kitlangton/tap">local path</a>' },
  '<meta http-equiv="refresh" content="0;url=https://example.org/;https://apparent.kitlangton.com/">',
  '<meta http-equiv="refresh" content="0;url https://apparent.kitlangton.com/">',
  '<code>echo kitlangton/tap/apparent/extra;</code>',
  '<code>echo path/kitlangton/tap/apparent;</code>',
  '<code>echo kitlangton/tapestry;</code>',
  '<code>echo kitlangton/tap/apparent?query;</code>',
]

// Run the real packager in a disposable site, never mutate the built artifact.
for (const [reject, cases] of [[true, rejected], [false, accepted]]) {
  for (const fragment of cases) {
    const { head = "", body = "" } = typeof fragment === "string" ? { body: fragment } : fragment
    test(`${reject ? "rejects" : "accepts"} ${head}${body}`, () => {
      const root = mkdtempSync(join(tmpdir(), "apparent-upstream-guard-"))
      try {
        cpSync(join(site, "scripts"), join(root, "scripts"), { recursive: true })
        writeFileSync(join(root, "package.json"), '{"type":"module"}')
        symlinkSync(join(site, "node_modules"), join(root, "node_modules"), "dir")
        const html = `<!doctype html><html><head><link rel="canonical" href="https://app.tryapparent.com/">${head}</head><body><h1>Visual feedback gets lost</h1><p>We believe work should be enjoyable</p>${body}</body></html>`
        for (const name of ["index.html", "connect", "hosted", "assets", "pond", "team", "feature-demos", "apparent-wordmark.svg", "icon.svg", "HostGrotesk-Variable.ttf", "social-preview-v1.png", "Apparent-ghost-demo-v4-lilac.mp4", "Apparent-ghost-demo-v4-lilac-poster.jpg"]) {
          const path = join(root, "dist", name)
          mkdirSync(dirname(path), { recursive: true })
          writeFileSync(path, name === "index.html" ? html : "synthetic asset")
        }
        mkdirSync(join(root, "dist-apparent"))
        writeFileSync(join(root, "dist-apparent", "sentinel"), "previous artifact")
        mkdirSync(join(root, "dist-apparent", "assets"))
        writeFileSync(join(root, "dist-apparent", "assets", "previous.bin"), Buffer.from([0, 255, 42]))
        const previousPaths = readdirSync(join(root, "dist-apparent"), { recursive: true }).sort()
        const result = spawnSync(process.execPath, [join(root, "scripts", "site-build", "package-apparent.mjs")], { encoding: "utf8" })
        if (reject) {
          assert.notEqual(result.status, 0)
          assert.match(result.stderr, /Refusing to publish upstream download or canonical links/)
          assert.equal(readFileSync(join(root, "dist-apparent", "sentinel"), "utf8"), "previous artifact")
          assert.deepEqual(readdirSync(join(root, "dist-apparent"), { recursive: true }).sort(), previousPaths)
          assert.deepEqual(readFileSync(join(root, "dist-apparent", "assets", "previous.bin")), Buffer.from([0, 255, 42]))
        } else {
          assert.equal(result.status, 0, result.stderr)
          assert.equal(readFileSync(join(root, "dist-apparent", "index.html"), "utf8"), html)
        }
      } finally {
        rmSync(root, { recursive: true, force: true })
      }
    })
  }
}

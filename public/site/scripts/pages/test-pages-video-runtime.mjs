import assert from "node:assert/strict"
import { test } from "node:test"
import { createRequire } from "node:module"
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { movie, chunkPath } from "../../lib/pages-video-metadata.js"

// Reuse the locked runtime installed by the owning workflow's hosted step.
const require = createRequire(new URL("../../../../connect/service/package.json", import.meta.url))
const { Miniflare } = require("miniflare")
const { build } = require("esbuild")

test("workerd transmits a fixed Content-Length for full and seeking movie responses", async () => {
  const original = readFileSync(new URL("../../public/assets/media/Apparent-ghost-demo-v4-lilac.mp4", import.meta.url))
  const bundled = await build({
    stdin: {
      contents: `import { onRequest } from "../../functions/Apparent-ghost-demo-v4-lilac.mp4.js";
        export default { fetch(request, env, ctx) {
          return onRequest({ request, env, waitUntil: ctx.waitUntil.bind(ctx) });
        } };`,
      resolveDir: fileURLToPath(new URL(".", import.meta.url)),
    },
    bundle: true, write: false, format: "esm", platform: "browser",
  })
  const runtime = new Miniflare({
    modules: true, script: bundled.outputFiles[0].text,
    compatibilityDate: "2026-08-06", host: "127.0.0.1", port: 0,
    serviceBindings: {
      ASSETS(request) {
        const index = movie.chunks.findIndex((chunk) => chunkPath(chunk) === new URL(request.url).pathname)
        assert.ok(index >= 0, "only approved chunk files may be read")
        // Real Pages can ignore Range and return the whole chunk.
        return new Response(original.subarray(index * movie.chunkSize, index * movie.chunkSize + movie.chunks[index].size))
      },
    },
  })
  try {
    const origin = await runtime.ready
    for (const [range, start, end] of [[null, 0, movie.size - 1], ["bytes=0-15", 0, 15], ["bytes=8388607-8388609", 8388607, 8388609], ["bytes=-128", movie.size - 128, movie.size - 1]]) {
      // Actual HTTP transport, not dispatchFetch: manually setting a header on
      // a Node Response hides workerd's unknown-length streaming behavior.
      const response = await fetch(new URL(movie.path, origin), { headers: range ? { Range: range } : {} })
      const bytes = Buffer.from(await response.arrayBuffer())
      assert.equal(response.status, range ? 206 : 200)
      assert.equal(response.headers.get("Content-Length"), String(end - start + 1), range ?? "full response")
      assert.deepEqual(bytes, original.subarray(start, end + 1))
      if (range) assert.equal(response.headers.get("Content-Range"), `bytes ${start}-${end}/${movie.size}`)
    }
  } finally {
    await runtime.dispose()
  }
})

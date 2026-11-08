import { defineConfig, type Plugin } from "vite"
import { fileURLToPath } from "node:url"
import { publicAssetPlugin } from "./lib/public-assets.mjs"
import react from "@vitejs/plugin-react"
import { agentFiles, jsonLd } from "./scripts/site-build/agent-files.ts"
import { MAP_DIR } from "./scripts/site-build/feature-audit.ts"

const TYPES: Record<string, string> = { ".txt": "text/plain; charset=utf-8", ".md": "text/markdown; charset=utf-8" }


// llms.txt, llms-full.txt and skill.md at the site root, plus the JSON-LD in index.html.
function forAgents(): Plugin {
  return {
    name: "apparent-agent-files",
    transformIndexHtml: () => [{ tag: "script", attrs: { type: "application/ld+json" }, children: jsonLd(), injectTo: "head" }],
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const name = req.url?.split("?")[0].slice(1) ?? ""
        const file = agentFiles()[name]
        if (file === undefined) return next()
        res.setHeader("Content-Type", TYPES[name.slice(name.lastIndexOf("."))])
        res.end(file)
      })
    },
    generateBundle() {
      if (this.environment?.config.build.ssr) return
      for (const [fileName, source] of Object.entries(agentFiles())) this.emitFile({ type: "asset", fileName, source })
    },
  }
}

// The feature map in dev; scripts/site-build/prerender.ts writes it for builds.
function featureMap(): Plugin {
  return {
    name: "apparent-feature-map",
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const path = req.url?.split("?")[0] ?? ""
        if (path !== `/${MAP_DIR}` && path !== `/${MAP_DIR}/`) return next()
        const { audit } = await server.ssrLoadModule("/scripts/site-build/feature-audit.ts")
        const { mapPage } = await server.ssrLoadModule("/src/map/page.tsx")
        res.setHeader("Content-Type", "text/html; charset=utf-8")
        res.end(mapPage(audit()))
      })
    },
  }
}

export default defineConfig({
  publicDir: false,
  // The public Apparent demo has no Kit release feed, agent instructions or feature map.
  plugins: [publicAssetPlugin(fileURLToPath(new URL("./public/assets/", import.meta.url))), react(), ...(process.env.APPARENT_PUBLIC_SITE === "1" ? [] : [forAgents(), featureMap()])],
  // three.js lives in the lazily loaded hero brush chunk.
  build: { chunkSizeWarningLimit: 700 },
  // Previews are shared over Tailscale (studio.*.ts.net).
  server: { allowedHosts: [".ts.net"] },
  preview: { allowedHosts: [".ts.net"] },
})

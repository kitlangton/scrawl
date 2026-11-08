import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"

export default defineConfig({
  appType: "mpa",
  plugins: [react(), {
    name: "connect-exact-routes",
    configureServer(server) {
      server.middlewares.use((req, _res, next) => {
        const url = new URL(req.url || "/", "http://localhost")
        if (/^\/(?:connect|hosted)(?:\/(?:device|shares))?\/?$/.test(url.pathname)) req.url = `/index.html${url.search}`
        next()
      })
    },
    configurePreviewServer(server) {
      server.middlewares.use((req, _res, next) => {
        const url = new URL(req.url || "/", "http://localhost")
        if (/^\/(?:connect|hosted)(?:\/(?:device|shares))?\/?$/.test(url.pathname)) req.url = `${url.pathname.replace(/\/$/, "")}/index.html${url.search}`
        next()
      })
    },
  }],
  build: { outDir: process.env.CONNECT_BUILD_DIR ?? "dist", sourcemap: false },
  // Same-site production split: browser on app.*, API/auth/callback on hosted.*.
  define: { "import.meta.env.VITE_HOSTED_API_ORIGIN": JSON.stringify(process.env.VITE_HOSTED_API_ORIGIN ?? "https://hosted.tryapparent.com") },
  server: { host: "127.0.0.1", allowedHosts: [".ts.net"] },
  preview: { host: "127.0.0.1", allowedHosts: [".ts.net"] },
})
import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"
export default defineConfig({
  plugins: [react()],
  publicDir: false,
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  build: {
    outDir: "dist-feature-grid",
    lib: { entry: "src/features/overlay-entry.tsx", formats: ["es"], fileName: "feature-grid", cssFileName: "feature-grid" },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
})

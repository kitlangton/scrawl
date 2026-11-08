import { StrictMode } from "react"
import { createRoot, hydrateRoot } from "react-dom/client"
import { App } from "./components/home/App"
import "./styles/styles.css"
import "./styles/pond-hero.css"
import "./cta.css"

const root = document.getElementById("root")!
const app = (
  <StrictMode>
    <App />
  </StrictMode>
)

// scripts/site-build/prerender.ts fills #root at build time; the dev server serves it empty.
if (root.firstElementChild) hydrateRoot(root, app)
else createRoot(root).render(app)

import { StrictMode } from "react"
import { createRoot, hydrateRoot } from "react-dom/client"
import { App } from "./components/home/App"
import "./styles/styles.css"
import "./styles/pond-hero.css"
import "./cta.css"
import "./styles/accepted-homepage.css"
import "./features/homepage-revision.css"
import "./styles/system-appearance.css"
import "./styles/approved-pond-layout.css"

const root = document.getElementById("root")!
const app = (
  <StrictMode>
    <App />
  </StrictMode>
)

// scripts/site-build/prerender.ts fills #root at build time; the dev server serves it empty.
if (root.firstElementChild) hydrateRoot(root, app)
else createRoot(root).render(app)

declare const __APPARENT_ANALYTICS_ENABLED__: boolean
declare const __APPARENT_ANALYTICS_CONFIG__: { enabled: boolean; token: string; host: string; environment: string }
if (__APPARENT_ANALYTICS_ENABLED__) void import("../../../shared/analytics/browser").then(({ startAnalytics }) => startAnalytics(__APPARENT_ANALYTICS_CONFIG__))

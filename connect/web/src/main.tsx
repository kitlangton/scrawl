import { StrictMode } from "react"
import { createRoot, hydrateRoot } from "react-dom/client"
import { Hosted, isHostedPath } from "./Hosted"
import "./base.css"
import { startAnalytics } from "../../../shared/analytics/browser"

const root = document.getElementById("root")!
const path = window.location.pathname
const app = <StrictMode>{isHostedPath(path) ? <Hosted devicePage={/^\/(?:connect|hosted)\/device\/?$/.test(path)} sharesPage={/^\/(?:connect|hosted)\/shares\/?$/.test(path)} /> : <p>Connect page not found.</p>}</StrictMode>
if (root.firstElementChild) hydrateRoot(root, app)
else createRoot(root).render(app)

declare const __APPARENT_ANALYTICS_ENABLED__: boolean
declare const __APPARENT_ANALYTICS_CONFIG__: { enabled: boolean; token: string; host: string; environment: string }
if (__APPARENT_ANALYTICS_ENABLED__) startAnalytics(__APPARENT_ANALYTICS_CONFIG__)
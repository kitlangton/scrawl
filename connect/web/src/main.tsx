import { StrictMode } from "react"
import { createRoot, hydrateRoot } from "react-dom/client"
import { Hosted, isHostedPath } from "./Hosted"
import "./base.css"

const root = document.getElementById("root")!
const path = window.location.pathname
const app = <StrictMode>{isHostedPath(path) ? <Hosted devicePage={/^\/(?:connect|hosted)\/device\/?$/.test(path)} sharesPage={/^\/(?:connect|hosted)\/shares\/?$/.test(path)} /> : <p>Connect page not found.</p>}</StrictMode>
if (root.firstElementChild) hydrateRoot(root, app)
else createRoot(root).render(app)
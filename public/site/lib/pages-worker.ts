// Advanced-mode Pages entry: compile once in verification, deploy without bundling.
// _routes.json restricts invocation to the approved movie URL; other requests
// retain Pages' static routing, redirects and privacy headers.
import { onRequest } from '../functions/Apparent-ghost-demo-v4-lilac.mp4.js'

export default {
  fetch(request: Request, env: { ASSETS: { fetch(request: Request): Promise<Response> } }) {
    return onRequest({ request, env })
  },
}

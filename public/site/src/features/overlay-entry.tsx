import "./homepage-revision.js"
import "./homepage-revision.css"
import { createRoot, type Root } from "react-dom/client"
import { FeatureGrid } from "./FeatureGrid"

// Mount after the already-approved homepage has hydrated. Keep its React root,
// photo/footer overlays, and all existing public assets intact.
let root: Root | undefined
let container: HTMLElement | undefined
function mount() {
  const demo = document.querySelector('.pond-demo')
  if (!demo || document.querySelector('.feature-grid-section')) return
  if (container?.isConnected) return
  root?.unmount()
  container = document.createElement('div')
  container.className = 'feature-grid-overlay-root'
  demo.after(container)
  root = createRoot(container)
  root.render(<FeatureGrid />)
}
function start() {
  mount()
  const observer = new MutationObserver(mount)
  observer.observe(document.querySelector('#root') ?? document.body, { childList: true, subtree: true })
  window.addEventListener('pagehide', () => observer.disconnect(), { once: true })
}
if (document.readyState === 'complete') setTimeout(start, 300)
else window.addEventListener('load', () => setTimeout(start, 300), { once: true })

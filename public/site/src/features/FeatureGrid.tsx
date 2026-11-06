import { useEffect, useRef, useState } from "react"
import "./feature-grid.css"
import demoTheme from "./demo-theme.css?raw"
import { MobileExample } from "./MobileExample"

const FEATURES = [
  { title: "Sketch what you mean", path: "how-it-works", description: "Sketch an idea on the whiteboard, finish recording, and show the change to your agent." },
  { title: "Every moment adds context", path: "timeline", description: "Voice, marks, clicks, and whiteboard sketches tell one story along a timeline." },
  { title: "Give your agent the full picture", path: "handoff", description: "Stop recording and paste file references, timestamped words, and interactions into your agent." },
  { title: "See what your agent changed", path: "anywhere", description: "Your agent shows its changes in a captioned walkthrough." },
]

function FeatureAnimation({ path, title, paused }: { path: string; title: string; paused: boolean }) {
  const host = useRef<HTMLDivElement>(null)
  const frame = useRef<HTMLIFrameElement>(null)
  const [width, setWidth] = useState(800)
  const [visible, setVisible] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [near, setNear] = useState(false)
  const prepareTheme = () => {
    const doc = frame.current?.contentDocument
    if (!doc?.head || !host.current) return
    let style = doc.getElementById("apparent-feature-theme")
    if (!style) {
      style = doc.createElement("style")
      style.id = "apparent-feature-theme"
      style.textContent = demoTheme
      // Example documents contain body styles; keep the theme after those rules.
      doc.body.append(style)
    }
    const theme = getComputedStyle(host.current)
    for (const role of ["background", "foreground", "primary", "primary-foreground", "border", "brand"])
      doc.documentElement.style.setProperty(`--demo-${role}`, theme.getPropertyValue(`--color-${role}`))
    doc.documentElement.style.setProperty("--demo-surface", theme.getPropertyValue("--color-muted"))
  }
  useEffect(() => {
    const element = host.current!
    const resize = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width))
    const preload = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) setNear(true)
    }, { rootMargin: "200px" })
    const visibility = new IntersectionObserver(([entry]) => setVisible(entry.isIntersecting), { threshold: .15 })
    resize.observe(element)
    preload.observe(element)
    visibility.observe(element)
    return () => { resize.disconnect(); preload.disconnect(); visibility.disconnect() }
  }, [])
  useEffect(() => {
    const phone = matchMedia("(max-width: 700px)")
    const update = () => frame.current?.contentWindow?.postMessage({ apparentFeature: paused || !visible || phone.matches ? "pause" : "play" }, location.origin)
    update()
    phone.addEventListener("change", update)
    return () => phone.removeEventListener("change", update)
  }, [paused, visible, loaded])
  useEffect(() => {
    const preference = matchMedia("(prefers-color-scheme: dark)")
    preference.addEventListener("change", prepareTheme)
    return () => preference.removeEventListener("change", prepareTheme)
  }, [])
  return <div className="feature-grid-viewport" ref={host}>
    <MobileExample path={path} />
    {near && <iframe ref={frame} aria-hidden="true" title={`${title} — illustrative animation`} src={`/feature-demos/${path}/index.html`}
      onLoad={() => { prepareTheme(); setLoaded(true) }} tabIndex={-1}
      style={{ transform: `scale(${width / 800})`, opacity: loaded ? 1 : 0 }} />}
  </div>
}

export function FeatureGrid() {
  const [paused, setPaused] = useState(false)
  useEffect(() => {
    const preference = matchMedia("(prefers-reduced-motion: reduce)")
    const update = () => setPaused(preference.matches)
    update()
    preference.addEventListener("change", update)
    return () => preference.removeEventListener("change", update)
  }, [])
  return <section className="feature-grid-section" aria-labelledby="feature-grid-title">
    <div className="feature-grid-heading">
      <h2 id="feature-grid-title">Make your feedback <img src="/apparent-wordmark.svg" alt="apparent" width={250} height={59} /></h2>
      <button type="button" className="feature-grid-toggle" onClick={() => setPaused(value => !value)} aria-pressed={paused}>
        {paused ? "Play animations" : "Pause animations"}
      </button>
    </div>
    <div className="feature-grid-cards">
      {FEATURES.map(feature => <article className="feature-grid-card" key={feature.path}>
        <FeatureAnimation path={feature.path} title={feature.path === "anywhere" ? "Agent walkthrough" : feature.title} paused={paused} />
        <div className="feature-copy"><h3>{feature.title}</h3>
        <p className="feature-grid-description">{feature.description}</p></div>
      </article>)}
    </div>
  </section>
}

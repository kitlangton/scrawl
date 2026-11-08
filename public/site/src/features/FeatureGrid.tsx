import { useEffect, useRef, useState } from "react"
import "./feature-grid.css"

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
    frame.current?.contentWindow?.postMessage({ apparentFeature: paused || !visible ? "pause" : "play" }, location.origin)
  }, [paused, visible, loaded])
  return <div className="feature-grid-viewport" ref={host}>
    {near && <iframe ref={frame} title={`${title} — illustrative animation`} src={`/feature-demos/${path}/index.html`}
      loading="lazy" onLoad={() => setLoaded(true)} tabIndex={-1}
      style={{ transform: `scale(${width / 800})` }} />}
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
        <FeatureAnimation path={feature.path} title={feature.title} paused={paused} />
        <h3>{feature.title}</h3>
        <p className="feature-grid-description">{feature.description}</p>
      </article>)}
    </div>
  </section>
}

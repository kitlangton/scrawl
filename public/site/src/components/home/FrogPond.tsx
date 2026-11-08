import { useEffect, useRef, useState } from "react"
import { LatestDownload } from "../download/LatestDownload"

const ART = "/pond"
// A prior 404 for this path is cached at the Cloudflare edge.
const LILY = `${ART}/lily-clean.png?v=1`

type Position = { x: number; y: number }

export function FrogPond() {
  const hero = useRef<HTMLElement>(null)
  const frog = useRef<HTMLButtonElement>(null)
  const landing = useRef<HTMLImageElement>(null)
  const position = useRef<Position>({ x: 0, y: 0 })
  const atEnd = useRef(false)
  const jumping = useRef(false)
  const waving = useRef(false)
  const timers = useRef<number[]>([])
  const activeAnimation = useRef<Animation | null>(null)
  const [landings, setLandings] = useState(0)
  const [waves, setWaves] = useState(0)
  const [airborne, setAirborne] = useState(false)
  const [greeting, setGreeting] = useState(false)
  const [wavePose, setWavePose] = useState(false)

  function queue(callback: () => void, delay: number) {
    timers.current.push(window.setTimeout(callback, delay))
  }

  function destination(): Position {
    const f = frog.current!.getBoundingClientRect()
    const p = landing.current!.getBoundingClientRect()
    return {
      x: p.left + p.width / 2 - (f.left - position.current.x) - f.width / 2,
      y: p.top + p.height * .48 - (f.top - position.current.y) - f.height * .82,
    }
  }

  function place(point: Position, facing: number) {
    position.current = point
    frog.current!.style.transform = `translate(${point.x}px, ${point.y}px) scaleX(${facing})`
  }

  function jump() {
    if (jumping.current || !frog.current) return
    const returning = atEnd.current
    const from = position.current
    const to = returning ? { x: 0, y: 0 } : destination()
    const facing = returning ? -1 : 1
    const f = frog.current
    jumping.current = true
    f.disabled = true
    place(from, facing) // Face the direction of travel before moving.
    const finish = () => {
      atEnd.current = !returning
      place(to, facing) // The frog stays left-facing after returning.
      if (!returning) place(destination(), facing) // A resize during flight moves the lily.
      setAirborne(false)
      jumping.current = false
      f.disabled = false
      f.setAttribute("aria-label", atEnd.current ? "Jump back to the first lily" : "Jump to the other lily")
      setLandings(count => count + 1)
    }
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      finish()
      return
    }
    const arc = Math.min(350, Math.max(220, Math.abs(to.y - from.y) * .85))
    const transform = (x: number, y: number, squash = "") => `translate(${x}px, ${y}px) scaleX(${facing}) ${squash}`
    const animation = f.animate([
      { transform: transform(from.x, from.y), offset: 0 },
      { transform: transform(from.x, from.y, "scaleY(.83)"), offset: .16 },
      { transform: transform(from.x + (to.x - from.x) * .23, from.y + (to.y - from.y) * .23 - arc * .72), offset: .35 },
      { transform: transform(from.x + (to.x - from.x) * .52, (from.y + to.y) / 2 - arc), offset: .54 },
      { transform: transform(from.x + (to.x - from.x) * .82, from.y + (to.y - from.y) * .82 - arc * .48), offset: .79 },
      { transform: transform(to.x, to.y, "scaleY(.84)"), offset: .93 },
      { transform: transform(to.x, to.y), offset: 1 },
    ], { duration: 970, easing: "linear" })
    activeAnimation.current = animation
    queue(() => setAirborne(true), 160)
    queue(() => setAirborne(false), 840)
    animation.finished.then(() => {
      if (activeAnimation.current !== animation) return
      activeAnimation.current = null
      finish()
    }, () => {
      if (activeAnimation.current !== animation) return
      activeAnimation.current = null
      jumping.current = false
      f.disabled = false
      setAirborne(false)
    })
  }

  function greet() {
    if (waving.current) return
    waving.current = true
    setWaves(count => count + 1)
    setGreeting(true)
    if (!matchMedia("(prefers-reduced-motion: reduce)").matches) queue(() => setWavePose(true), 120)
    queue(() => setWavePose(false), 620)
    queue(() => { setGreeting(false); waving.current = false }, 950)
  }

  useEffect(() => {
    const observer = new ResizeObserver(() => {
      if (atEnd.current && !jumping.current && frog.current) place(destination(), 1)
    })
    if (hero.current) observer.observe(hero.current)
    return () => {
      observer.disconnect()
      for (const timer of timers.current) clearTimeout(timer)
      activeAnimation.current?.cancel()
      activeAnimation.current = null
    }
  }, [])

  return <section className="pond-hero" ref={hero} id="top">
    <div className="pond-copy">
      <h1>Visual feedback gets lost<br />in <span className="pond-word"><span className="pond-word-text">text.</span><img className="pond-cover-pad" src={LILY} alt="" /><button ref={frog} className="pond-cover-frog" type="button" onClick={jump} aria-label="Jump to the other lily" data-at-end={atEnd.current} data-landings={landings}><img src={`${ART}/frog-rest-cohesive.png`} alt="" className={airborne ? "pond-hidden" : ""} /><img src={`${ART}/frog-air-cohesive.png`} alt="" className={airborne ? "" : "pond-hidden"} /></button></span></h1>
      <p><img className="pond-wordmark" src="/apparent-wordmark.svg" alt="Apparent" /> lets you show an agent what to change and see what it made.</p>
      <div className="pond-download"><LatestDownload /></div>
    </div>
    <div className="pond-scene">
      <img className="pond-pad pond-left-pad" src={LILY} alt="" />
      <button type="button" className="pond-lounging-frog" onClick={greet} aria-label="Wave to the lounging frog" data-waves={waves}><img src={`${ART}/frog-lounge-cohesive.png`} alt="" className={wavePose ? "pond-hidden" : ""} /><img src={`${ART}/frog-wave-cohesive.png`} alt="" className={wavePose ? "" : "pond-hidden"} /><span className="pond-hello" aria-hidden="true" data-visible={greeting}>hi!</span></button>
      <img ref={landing} className="pond-pad pond-right-pad" src={LILY} alt="" />
    </div>
  </section>
}

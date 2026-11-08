import { useEffect, useRef, useState } from "react"
import type { PointerEvent as ReactPointerEvent, KeyboardEvent } from "react"
import "./video-demo.css"

const SOURCE = "/Apparent-ghost-demo-v4-lilac.mp4"
const POSTER = "/Apparent-ghost-demo-v4-lilac-poster.jpg"
const DURATION = 3589 / 60
// Seconds in the approved final export, not in the original recording.
const CHAPTERS = [
  { label: "Start", t: 0 },
  { label: "Capture", t: 3.7333333333333334 },
  { label: "Sketch", t: 15.433333333333334 },
  { label: "Handoff", t: 27.92 },
  { label: "Result", t: 48.7 },
  { label: "Try it", t: 55.7 },
]

export function Demo() {
  const video = useRef<HTMLVideoElement>(null)
  const track = useRef<HTMLDivElement>(null)
  const pendingSeek = useRef<number | null>(null)
  const playRequest = useRef(0)
  const scrubbing = useRef(false)
  const [time, setTime] = useState(0)
  const [duration, setDuration] = useState(DURATION)
  const [playing, setPlaying] = useState(false)
  const [started, setStarted] = useState(false)
  const [sound, setSound] = useState(false)
  const [error, setError] = useState("")
  // Media clocks round to microseconds; don't select the previous chapter at its first frame.
  const current = CHAPTERS.reduce((index, chapter, i) => time + .001 >= chapter.t ? i : index, 0)
  const ended = started && time >= duration - .08 && !playing

  useEffect(() => {
    const element = video.current
    return () => {
      playRequest.current++
      element?.pause()
    }
  }, [])

  const pause = () => { playRequest.current++; video.current?.pause() }
  const play = (audible = sound) => {
    const element = video.current
    if (!element) return
    const request = ++playRequest.current
    element.muted = !audible
    setSound(audible)
    setError("")
    // Call play in the user gesture, not an effect, so sound works on mobile too.
    void element.play().catch((reason: unknown) => {
      if (request !== playRequest.current) return
      if (reason instanceof DOMException && reason.name === "AbortError") return
      setError("Playback could not start. Try Play again, or open the video directly.")
    })
  }
  const seek = (seconds: number) => {
    const element = video.current
    if (!element) return
    const next = Math.max(0, Math.min(duration, seconds))
    setTime(next)
    if (element.readyState >= 1) element.currentTime = next
    else {
      pendingSeek.current = next
      element.load()
    }
  }
  const replay = () => { seek(0); play() }
  const toggle = () => {
    if (video.current && !video.current.paused) pause()
    else { if (ended) seek(0); play() }
  }
  const chapter = (seconds: number) => { seek(seconds); play() }
  const seekPointer = (event: ReactPointerEvent) => {
    const bounds = track.current?.getBoundingClientRect()
    if (bounds?.width) seek((event.clientX - bounds.left) / bounds.width * duration)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // Native focused buttons handle their own Space/Enter; don't toggle twice.
    if (event.target !== event.currentTarget) return
    if (event.key === " ") { event.preventDefault(); toggle() }
    else if (event.key === "ArrowRight") {
      event.preventDefault(); seek(CHAPTERS[Math.min(CHAPTERS.length - 1, current + 1)].t)
    } else if (event.key === "ArrowLeft") {
      event.preventDefault(); seek(time - CHAPTERS[current].t > .7 ? CHAPTERS[current].t : CHAPTERS[Math.max(0, current - 1)].t)
    }
  }

  return (
    <div className="demo video-demo">
      <div className="stage-wrap" tabIndex={0} aria-label="Apparent demo" onKeyDown={onKeyDown}>
        <video ref={video} src={SOURCE} poster={POSTER} preload="none" playsInline muted={!sound}
          aria-label="A real Apparent capture: launch, sketch, handoff and the implemented ghost"
          onLoadedMetadata={() => {
            const element = video.current!
            if (Number.isFinite(element.duration) && element.duration > 0) setDuration(element.duration)
            if (pendingSeek.current !== null) { element.currentTime = pendingSeek.current; pendingSeek.current = null }
          }}
          onTimeUpdate={() => setTime(video.current?.currentTime ?? 0)}
          onPlay={() => { setPlaying(true); setStarted(true) }} onPause={() => setPlaying(false)}
          onEnded={() => { setPlaying(false); setTime(video.current?.duration ?? DURATION) }}
          onError={() => setError("The demo could not load. Try again, or open the video directly.")} />
        {!started && <button className="stage-play liquid-glass" onClick={() => play(true)}><PlayIcon />Play with sound</button>}
        {started && playing && !sound && <button className="stage-sound-hint liquid-glass" onClick={() => { if (video.current) video.current.muted = false; setSound(true) }}>Tap for sound</button>}
        {ended && <div className="stage-end"><button className="stage-btn liquid-glass" onClick={replay}><ReplayIcon />Replay</button></div>}
      </div>
      {error && <p role="alert">{error} <a href={SOURCE}>Open video</a></p>}
      <div className="controls">
        <div className="controls-left">
          <button className="icon-btn" onClick={toggle} aria-label={playing ? "Pause" : "Play"}>{playing ? <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="2.5" width="3" height="11" rx="1" /><rect x="9.5" y="2.5" width="3" height="11" rx="1" /></svg> : <PlayIcon />}</button>
          <button className="icon-btn" onClick={replay} aria-label="Replay"><ReplayIcon /></button>
          <button className={`sound-btn${sound ? " on" : ""}${sound && playing ? " live" : ""}`} aria-pressed={sound} aria-label={sound ? "Mute sound" : "Enable sound"}
            onClick={() => { if (video.current) video.current.muted = sound; setSound(!sound) }}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5z" fill="currentColor" />{sound ? <><path className="sound-wave w1" d="M10.5 5.5a3.5 3.5 0 010 5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" /><path className="sound-wave w2" d="M12.3 3.7a6 6 0 010 8.6" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" /></> : <path d="M10.5 6l4 4M14.5 6l-4 4" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />}</svg>
            <span className="sound-label">{sound ? "Sound" : "Muted"}</span>
          </button>
        </div>
        <div className="scrubber">
          <div ref={track} className="track" role="slider" tabIndex={0} aria-label="Demo timeline" aria-valuemin={0} aria-valuemax={duration} aria-valuenow={time} aria-valuetext={`${Math.floor(time)} of ${Math.ceil(duration)} seconds`}
            onKeyDown={(event) => {
              if (event.key === "ArrowRight" || event.key === "ArrowLeft") { event.preventDefault(); seek(time + (event.key === "ArrowRight" ? 5 : -5)) }
              else if (event.key === "Home" || event.key === "End") { event.preventDefault(); seek(event.key === "Home" ? 0 : duration) }
            }}
            onPointerDown={(event) => { if (event.button !== 0) return; scrubbing.current = true; event.currentTarget.setPointerCapture(event.pointerId); pause(); seekPointer(event) }}
            onPointerMove={(event) => { if (scrubbing.current) seekPointer(event) }}
            onPointerUp={() => { scrubbing.current = false }} onPointerCancel={() => { scrubbing.current = false }} onLostPointerCapture={() => { scrubbing.current = false }}>
            {CHAPTERS.map((item, i) => { const end = CHAPTERS[i + 1]?.t ?? duration; const fill = Math.min(1, Math.max(0, (time - item.t) / (end - item.t))); return <div key={item.label} className="seg" style={{ flexGrow: end - item.t }}><i style={{ transform: `scaleX(${fill})` }} /></div> })}
          </div>
          <div className="beats">{CHAPTERS.map((item, i) => <button key={item.label} className={i === current ? "on" : ""} aria-label={`Chapter: ${item.label}`} aria-current={i === current ? "step" : undefined} style={{ flexGrow: (CHAPTERS[i + 1]?.t ?? duration) - item.t }} onClick={() => chapter(item.t)}><span>{i + 1}</span><em>{item.label}</em></button>)}</div>
        </div>
      </div>
    </div>
  )
}

const PlayIcon = () => <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 2.8v10.4a.6.6 0 00.9.5l8.3-5.2a.6.6 0 000-1L5.4 2.3a.6.6 0 00-.9.5z" /></svg>
const ReplayIcon = () => <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden="true"><path d="M3 8a5 5 0 105-5H5.5" /><path d="M7 1L5 3l2 2" /></svg>

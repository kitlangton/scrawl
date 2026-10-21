import { type ComponentType, type CSSProperties, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react"
import { flushSync } from "react-dom"
import { MacWindow, PhoneMirror } from "./apps"
import type { StageGpuOverlay as StageGpuOverlayType } from "./gpu-fx"
import { afterLoad, useAfterLoad, useHydrated } from "../idle"
import { INK_SHADOW, InkPath, inkPath } from "./ink"
import { measureAnchors, sameAnchors } from "./measure"
import { motion, motionControls, motionServer } from "./motion"
import { AGENT, type Anchors, BEATS, buildScript, cursorAt, type Mode, STAGE, T } from "./script"
import * as sfx from "./sfx"
import { Caption, Cursor, EdgeGlow, fixState, Island, Ink, Keys, MenuBar, phoneLive, Ripples, terminalActive } from "./stage-parts"
import { Terminal } from "./terminal"

type UserStroke = { points: [number, number, number][]; done: boolean }

const supportsSvgBackdrop =
  typeof window !== "undefined" &&
  typeof CSS !== "undefined" &&
  CSS.supports("backdrop-filter", "url(#scrawl-liquid-glass)") &&
  /Chrome|Chromium|CriOS|Edg/i.test(navigator.userAgent) &&
  !("GestureEvent" in window)

function createLensDataUrl(): string | null {
  if (!supportsSvgBackdrop || typeof document === "undefined") return null
  const w = 180
  const h = 52
  const canvas = document.createElement("canvas")
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext("2d")
  if (!ctx) return null
  const img = ctx.createImageData(w, h)
  const d = img.data
  const r = h / 2
  const bevel = 15
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const cx = Math.max(r, Math.min(w - r, x))
      const cy = r
      const dx = x - cx
      const dy = y - cy
      const dist = Math.hypot(dx, dy)
      const edgeDist = r - dist
      let nx = 0
      let ny = 0
      if (edgeDist > 0 && edgeDist < bevel && dist > 1e-4) {
        const t = 1 - edgeDist / bevel
        const mag = t * t
        nx = (dx / dist) * mag
        ny = (dy / dist) * mag
      }
      const idx = (y * w + x) * 4
      d[idx] = Math.round(128 - nx * 124)
      d[idx + 1] = Math.round(128 - ny * 124)
      d[idx + 2] = 128
      d[idx + 3] = 255
    }
  }
  ctx.putImageData(img, 0, 0)
  return canvas.toDataURL()
}

const LENS_DATA_URL = createLensDataUrl()

declare global {
  interface Window {
    __scrawlDemo?: {
      ready: () => boolean
      t: () => number
      seek: (ms: number) => void
      mode: (m: Mode) => void
      play: () => void
    }
  }
}

// `?timescale=0.25` plays the timeline in slow motion (narration off) to judge motion by eye.
const TIMESCALE = (() => {
  if (typeof window === "undefined") return 1
  const v = Number(new URLSearchParams(window.location.search).get("timescale"))
  return v > 0 && v <= 4 ? v : 1
})()

// `stepClock` advances t each animation frame and locks to the WebAudio narration clock while it plays.
function usePlayback(duration: number, stepClock: (prevT: number, dt: number) => number, onSync: (t: number, playing: boolean) => void) {
  const [t, setT] = useState(0)
  const [playing, setPlaying] = useState(false)
  const tRef = useRef(0)
  const playingRef = useRef(false)
  const stepRef = useRef(stepClock)
  const syncRef = useRef(onSync)
  stepRef.current = stepClock
  syncRef.current = onSync
  playingRef.current = playing

  useEffect(() => {
    if (!playing) {
      syncRef.current(tRef.current, false)
      return
    }
    let raf = 0
    let last = performance.now()
    const tick = (now: number) => {
      const dt = document.hidden ? 0 : Math.min(200, Math.max(0, now - last))
      last = now
      if (TIMESCALE !== 1) for (const a of document.getAnimations()) a.playbackRate = TIMESCALE
      const next = Math.min(duration, stepRef.current(tRef.current, dt))
      tRef.current = next
      setT(next)
      if (next >= duration) {
        playingRef.current = false
        setPlaying(false)
        syncRef.current(next, false)
      } else {
        raf = requestAnimationFrame(tick)
      }
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [playing, duration])

  const seek = useCallback(
    (v: number) => {
      const next = Math.max(0, Math.min(duration, v))
      tRef.current = next
      setT(next)
      syncRef.current(next, playingRef.current)
    },
    [duration],
  )

  const play = useCallback(() => {
    if (tRef.current >= duration) {
      tRef.current = 0
      setT(0)
    }
    playingRef.current = true
    setPlaying(true)
    syncRef.current(tRef.current, true)
  }, [duration])

  const pause = useCallback(() => {
    playingRef.current = false
    setPlaying(false)
    syncRef.current(tRef.current, false)
  }, [])

  return { t, tRef, playing, setPlaying, seek, play, pause }
}

const beatIndex = (t: number) => {
  let i = 0
  BEATS.forEach((b, j) => {
    if (t >= b.t) i = j
  })
  return i
}

export function Demo() {
  const [mode, setMode] = useState<Mode>("mac")
  const [sound, setSound] = useState(true)
  const [started, setStarted] = useState(false)
  const [soundPulse, setSoundPulse] = useState(false)

  const stepClock = useCallback(
    (prevT: number, dt: number) => {
      const candidate = prevT + dt * TIMESCALE
      if (TIMESCALE !== 1) return candidate
      const master = sfx.syncNarration(mode, candidate, true, sound)
      return master !== null ? master : candidate
    },
    [mode, sound],
  )

  const onSync = useCallback(
    (curT: number, isPlaying: boolean) => {
      sfx.syncNarration(mode, curT, isPlaying, sound)
    },
    [mode, sound],
  )

  const { t, tRef, playing, seek, play, pause } = usePlayback(T.end, stepClock, onSync)
  const scriptRef = useRef<ReturnType<typeof buildScript> | null>(null)
  const wrapRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  const badgeRef = useRef<HTMLSpanElement>(null)
  const actionsRef = useRef<HTMLDivElement>(null)
  const titleRef = useRef<HTMLHeadingElement>(null)
  const [scale, setScale] = useState(1)
  const lens = useHydrated() ? LENS_DATA_URL : null
  const ready = useAfterLoad(wrapRef)
  const [anchors, setAnchors] = useState<Anchors | null>(null)
  const [user, setUser] = useState<UserStroke[]>([])
  const [shiftDown, setShiftDown] = useState(false)
  const [drawMode, setDrawMode] = useState(false)
  const [GpuOverlay, setGpuOverlay] = useState<ComponentType<Parameters<typeof StageGpuOverlayType>[0]> | null>(null)

  // Lazy-load the wet-ink sheen after first paint so initial load stays untouched. Phones draw the
  // stage too small for it to show, so they never load it.
  useEffect(() => {
    if (window.innerWidth < 700) return
    let cancelled = false
    const cancel = afterLoad(() => {
      import("./gpu-fx").then((mod) => {
        if (!cancelled) setGpuOverlay(() => mod.StageGpuOverlay)
      })
    })
    return () => {
      cancelled = true
      cancel()
    }
  }, [])

  useEffect(() => {
    const cancel = afterLoad(() => sfx.preloadNarration())
    return () => {
      cancel()
      sfx.stopNarration()
    }
  }, [])

  useEffect(() => {
    sfx.setEnabled(sound)
    sfx.syncNarration(mode, tRef.current, playing, sound)
  }, [sound, mode, playing, tRef])

  const triggerSoundPulse = useCallback(() => {
    setSoundPulse(true)
    const id = window.setTimeout(() => setSoundPulse(false), 900)
    return () => clearTimeout(id)
  }, [])

  const start = (withSound = sound, cue: sfx.Sound | null = null) => {
    sfx.setEnabled(withSound)
    sfx.unlock()
    if (withSound && cue) {
      sfx.play(cue, 0.9)
      triggerSoundPulse()
    }
    setStarted(true)
    setSound(withSound)
    play()
  }

  // A click on the page's own controls.
  const click = (s: sfx.Sound = "tick") => {
    sfx.unlock()
    sfx.play(s, 0.8)
  }

  useLayoutEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const update = () => {
      const w = el.clientWidth
      if (w > 0) setScale(w / STAGE.w)
    }
    update()
    const ro = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width > 0) setScale(entry.contentRect.width / STAGE.w)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const look = useSyncExternalStore(motionControls.subscribe, motion, motionServer)
  // CSS-driven reveals (blocks entering the terminal, diff lines, contact-sheet cards) read these.
  const lookVars = {
    "--m-enter-ms": `${look.enterMs}ms`,
    "--m-enter-rise": `${look.enterRise}px`,
    "--m-scroll-ms": `${look.scrollMs}ms`,
  } as CSSProperties

  // Dev server only: hover the stage for the ⚙ knobs chip with every motion value and the looks.
  useEffect(() => {
    const host = wrapRef.current
    if (!host || !motionControls.enabled) return
    return motionControls.attach(host)
  }, [])

  const fixNow = fixState(t)
  // One object per distinct fix state, so the memoized app windows skip frames where nothing moved.
  const fix = useMemo(() => fixNow, [fixNow.badge, fixNow.rows, fixNow.actions, fixNow.badgeSheen, fixNow.actionsSheen]) // eslint-disable-line react-hooks/exhaustive-deps
  const broken = t < T.fix[0]
  const fixRef = useRef(broken)
  fixRef.current = broken

  // Strokes target the broken layout, so only measure while nothing has been fixed yet.
  useLayoutEffect(() => {
    if (!broken) return
    const measure = () => {
      if (!fixRef.current || !stageRef.current || !badgeRef.current || !actionsRef.current || !titleRef.current) return
      const next = measureAnchors(stageRef.current, badgeRef.current, actionsRef.current, titleRef.current)
      setAnchors((prev) => (prev && sameAnchors(prev, next) ? prev : next))
    }
    measure()
    document.fonts?.ready.then(measure)
  }, [mode, broken, ready])

  useEffect(() => {
    const down = (e: PointerEvent) => {
      const btn = (e.target as Element | null)?.closest?.(".liquid-glass")
      if (btn) btn.classList.add("pressed")
    }
    const clear = () => {
      for (const el of document.querySelectorAll(".liquid-glass.pressed")) {
        el.classList.remove("pressed")
      }
    }
    window.addEventListener("pointerdown", down, true)
    window.addEventListener("pointerup", clear, true)
    window.addEventListener("pointercancel", clear, true)
    window.addEventListener("blur", clear)
    return () => {
      window.removeEventListener("pointerdown", down, true)
      window.removeEventListener("pointerup", clear, true)
      window.removeEventListener("pointercancel", clear, true)
      window.removeEventListener("blur", clear)
    }
  }, [])
  useEffect(() => {
    const key = (e: KeyboardEvent) => setShiftDown(e.shiftKey)
    const blur = () => setShiftDown(false)
    window.addEventListener("keydown", key)
    window.addEventListener("keyup", key)
    window.addEventListener("blur", blur)
    return () => {
      window.removeEventListener("keydown", key)
      window.removeEventListener("keyup", key)
      window.removeEventListener("blur", blur)
    }
  }, [])

  const script = useMemo(() => (anchors ? buildScript(mode, anchors) : null), [anchors, mode])
  scriptRef.current = script

  // Test hook for motion captures: seek the timeline to any ms and render it synchronously, so a
  // capture can step at a fixed 60 fps of demo time regardless of the browser's frame rate.
  useEffect(() => {
    window.__scrawlDemo = {
      ready: () => !!scriptRef.current,
      t: () => tRef.current,
      seek: (ms: number) => {
        flushSync(() => {
          setStarted(true)
          pause()
          seek(ms)
        })
      },
      mode: (next: Mode) => flushSync(() => setMode(next)),
      play: () => {
        setStarted(true)
        setSound(false)
        sfx.setEnabled(false)
        play()
      },
    }
  }, [pause, play, seek, tRef])

  const switchMode = (next: Mode) => {
    if (next === mode) return
    sfx.stopNarration()
    click("toggle")
    setUser([])
    seek(0)
    setMode(next)
    start(sound)
  }

  const replay = () => {
    sfx.stopNarration()
    click("pulse")
    setUser([])
    seek(0)
    start(sound)
  }

  const toStage = (e: React.PointerEvent): [number, number, number] => {
    const r = stageRef.current!.getBoundingClientRect()
    return [(e.clientX - r.left) / scale, (e.clientY - r.top) / scale, e.pointerType === "pen" ? e.pressure : 0.5]
  }

  const pen = useRef<{ x: number; y: number; at: number; speed: number } | null>(null)

  const onPointerDown = (e: React.PointerEvent) => {
    if (!(e.shiftKey || drawMode || e.pointerType === "pen")) return
    e.preventDefault()
    ;(e.currentTarget as Element).setPointerCapture?.(e.pointerId)
    sfx.unlock()
    pen.current = { x: e.clientX, y: e.clientY, at: e.timeStamp, speed: 0 }
    setUser((s) => [...s, { points: [toStage(e)], done: false }])
  }
  const onPointerMove = (e: React.PointerEvent) => {
    const p = pen.current
    if (!p) return
    const dt = Math.max(1, e.timeStamp - p.at)
    const v = Math.hypot(e.clientX - p.x, e.clientY - p.y) / scale / dt
    p.speed += (v - p.speed) * 0.35
    pen.current = { x: e.clientX, y: e.clientY, at: e.timeStamp, speed: p.speed }
    sfx.scribble("user", "marker", p.speed / 0.9)
    setUser((s) => {
      const last = s[s.length - 1]
      if (!last || last.done) return s
      return [...s.slice(0, -1), { ...last, points: [...last.points, toStage(e)] }]
    })
  }
  const onPointerUp = () => {
    if (pen.current) sfx.scribbleStop("user")
    pen.current = null
    setUser((s) => {
      const last = s[s.length - 1]
      if (!last || last.done) return s
      return [...s.slice(0, -1), { ...last, done: true }]
    })
  }

  const onKeyDown = (e: React.KeyboardEvent) => {
    const i = beatIndex(t)
    if (e.key === "ArrowRight") {
      e.preventDefault()
      seek(BEATS[Math.min(BEATS.length - 1, i + 1)].t)
    } else if (e.key === "ArrowLeft") {
      e.preventDefault()
      seek(t - BEATS[i].t > 700 ? BEATS[i].t : BEATS[Math.max(0, i - 1)].t)
    } else if (e.key === " ") {
      e.preventDefault()
      if (playing) {
        click("press")
        pause()
      } else {
        start(sound, !started && sound ? "start" : "press")
      }
    } else if (e.key === "z" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      setUser((s) => s.slice(0, -1))
    }
  }

  const cursor = script ? cursorAt(script, t) : null
  const live = mode === "iphone" ? phoneLive(t) : 0
  const refs = useMemo(() => ({ badge: badgeRef, actions: actionsRef, title: titleRef }), [])
  const drawing = shiftDown || drawMode
  const cues = useMemo(() => sfx.demoCues(mode), [mode])
  const last = useRef<{ t: number; at: [number, number] | null; speed: number }>({ t: 0, at: null, speed: 0 })

  // Scripted cues fire as the clock crosses them; seeks and scrubs (large or backward jumps) stay quiet.
  useEffect(() => {
    const prev = last.current
    const at = cursor?.at ?? null
    const dt = t - prev.t
    const stepping = playing && sound && dt > 0 && dt < 250
    if (stepping) for (const c of cues) if (c.t > prev.t && c.t <= t) sfx.play(c.sound, c.volume)
    const texture: sfx.Texture | null = stepping && cursor?.drawing ? "marker" : null
    const v = texture && at && prev.at ? Math.hypot(at[0] - prev.at[0], at[1] - prev.at[1]) / dt : 0
    const speed = texture ? prev.speed + (v - prev.speed) * 0.4 : 0
    if (texture) sfx.scribble("demo", texture, speed / (texture === "marker" ? 0.45 : 1.4))
    else sfx.scribbleStop("demo")
    last.current = { t, at, speed }
  }, [t, playing, sound, cues, cursor, mode])

  const ended = started && !playing && t >= T.end - 50
  const other: Mode = mode === "mac" ? "iphone" : "mac"

  const overTerminal =
    !!cursor && cursor.at[0] > AGENT.x && cursor.at[0] < AGENT.x + AGENT.w && cursor.at[1] > AGENT.y + 28 && cursor.at[1] < AGENT.y + AGENT.h

  return (
    <div className={`demo${lens ? " svg-glass" : ""}`} style={lookVars}>
      {lens && (
        <svg className="liquid-glass-defs" aria-hidden>
          <defs>
            <filter
              id="scrawl-liquid-glass"
              x="0%"
              y="0%"
              width="100%"
              height="100%"
              filterUnits="objectBoundingBox"
              primitiveUnits="objectBoundingBox"
              colorInterpolationFilters="sRGB"
            >
              <feGaussianBlur in="SourceGraphic" stdDeviation="0.06 0.2" edgeMode="duplicate" result="preblur" />
              <feImage href={lens} x="0" y="0" width="1" height="1" preserveAspectRatio="none" result="lens" />
              <feDisplacementMap in="preblur" in2="lens" scale="0.075" xChannelSelector="R" yChannelSelector="G" result="refracted" />
              <feGaussianBlur in="refracted" stdDeviation="0.015 0.05" edgeMode="duplicate" result="blurred" />
              <feColorMatrix in="blurred" type="saturate" values="2.05" result="saturated" />
              <feComponentTransfer in="saturated">
                <feFuncR type="linear" slope="1.22" intercept="0.02" />
                <feFuncG type="linear" slope="1.22" intercept="0.02" />
                <feFuncB type="linear" slope="1.24" intercept="0.025" />
              </feComponentTransfer>
            </filter>
          </defs>
        </svg>
      )}
      <ModeSwitch mode={mode} onMode={switchMode} />
      <div
        ref={wrapRef}
        className={`stage-wrap${drawing ? " drawing" : ""}`}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        aria-label="Scrawl demo"
      >
        <div
          ref={stageRef}
          className="stage wallpaper"
          style={{ width: STAGE.w, height: STAGE.h, transform: `scale(${scale})` }}
          aria-hidden
          inert
        >
          {ready && (
            <div className="stage-live">
              <MenuBar t={t} />
              {mode === "mac" ? (
                <MacWindow fix={fix} sheen={look.sheen} refs={refs} inactive={terminalActive(t)} />
              ) : (
                <PhoneMirror fix={fix} sheen={look.sheen} refs={refs} live={live} />
              )}
              {script && <Terminal t={t} mode={mode} script={script} />}
              <EdgeGlow t={t} />
              {script && <Ink t={t} script={script} />}
              {script && <Caption t={t} script={script} />}
              <Keys t={t} mode={mode} />
              <Island t={t} mode={mode} />
              {script && <Ripples t={t} script={script} />}
              {user.length > 0 && (
                <svg className="ink" viewBox={`0 0 ${STAGE.w} ${STAGE.h}`} style={INK_SHADOW}>
                  {user.map((s, i) => (
                    <InkPath key={i} d={inkPath(s.points, 4.5, 0.5)} />
                  ))}
                </svg>
              )}
              {GpuOverlay && <GpuOverlay t={t} script={script} intensity={look.inkSheen} stageRef={stageRef} />}
              {cursor && <Cursor c={cursor} ibeam={overTerminal && t >= T.promptClick - 400 && t < T.paste + 120} />}
            </div>
          )}
        </div>
        {!started && (
          <button
            className="stage-play liquid-glass"
            onPointerDown={(e) => {
              e.stopPropagation()
              sfx.unlock()
            }}
            onClick={() => start(true, "start")}
          >
            <svg viewBox="0 0 16 16" aria-hidden>
              <path d="M4.5 2.8v10.4a.6.6 0 00.9.5l8.3-5.2a.6.6 0 000-1L5.4 2.3a.6.6 0 00-.9.5z" />
            </svg>
            Play with sound
          </button>
        )}
        {started && playing && !sound && (
          <button
            className="stage-sound-hint liquid-glass"
            onPointerDown={(e) => {
              e.stopPropagation()
              sfx.unlock()
            }}
            onClick={() => {
              sfx.setEnabled(true)
              sfx.unlock()
              sfx.play("start", 0.85)
              triggerSoundPulse()
              setSound(true)
            }}
          >
            <svg viewBox="0 0 16 16" aria-hidden>
              <path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5z" fill="currentColor" />
              <path d="M10.5 6l4 4M14.5 6l-4 4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
            Tap for sound
          </button>
        )}
        {ended && (
          <div className="stage-end" onPointerDown={(e) => e.stopPropagation()}>
            <button className="stage-btn liquid-glass" onClick={replay}>
              <ReplayIcon />
              Replay
            </button>
            <button className="stage-btn primary liquid-glass liquid-glass-primary" onClick={() => switchMode(other)}>
              {other === "iphone" ? <PhoneIcon /> : <MacIcon />}
              {other === "iphone" ? "Watch iPhone Mirror" : "Watch Mac Screen"}
              <span aria-hidden>→</span>
            </button>
          </div>
        )}
      </div>
      <Controls
        t={t}
        playing={playing}
        drawMode={drawMode}
        sound={sound}
        soundPulse={soundPulse}
        onToggle={() => {
          if (playing) {
            click("press")
            pause()
          } else {
            start(sound, !started && sound ? "start" : "press")
          }
        }}
        onSound={() => {
          const next = !sound
          sfx.setEnabled(next)
          sfx.unlock()
          if (next) {
            sfx.play("start", 0.75)
            triggerSoundPulse()
          }
          setSound(next)
        }}
        onReplay={replay}
        onSeek={seek}
        onBeat={(bt) => {
          click("tick")
          seek(bt)
          start(sound)
        }}
        onDraw={() => {
          click("toggle")
          setDrawMode((d) => !d)
        }}
        onPause={pause}
      />
    </div>
  )
}

function Controls(props: {
  t: number
  playing: boolean
  drawMode: boolean
  sound: boolean
  soundPulse: boolean
  onToggle: () => void
  onSound: () => void
  onReplay: () => void
  onSeek: (t: number) => void
  onBeat: (t: number) => void
  onDraw: () => void
  onPause: () => void
}) {
  const { t, playing, drawMode, sound, soundPulse } = props
  const trackRef = useRef<HTMLDivElement>(null)
  const scrubbing = useRef(false)

  const seekFromEvent = (e: React.PointerEvent) => {
    const r = trackRef.current!.getBoundingClientRect()
    props.onSeek(((e.clientX - r.left) / r.width) * T.end)
  }

  const current = beatIndex(t)

  return (
    <div className="controls">
      <div className="controls-left">
        <button className="icon-btn" onClick={props.onToggle} aria-label={playing ? "Pause" : "Play"}>
          {playing ? (
            <svg viewBox="0 0 16 16">
              <rect x="3.5" y="2.5" width="3" height="11" rx="1" />
              <rect x="9.5" y="2.5" width="3" height="11" rx="1" />
            </svg>
          ) : (
            <svg viewBox="0 0 16 16">
              <path d="M4.5 2.8v10.4a.6.6 0 00.9.5l8.3-5.2a.6.6 0 000-1L5.4 2.3a.6.6 0 00-.9.5z" />
            </svg>
          )}
        </button>
        <button className="icon-btn" onClick={props.onReplay} aria-label="Replay">
          <ReplayIcon />
        </button>
        <button
          className={`sound-btn${sound ? " on" : ""}${sound && playing ? " live" : ""}${soundPulse ? " pulse" : ""}`}
          onClick={props.onSound}
          aria-pressed={sound}
          aria-label={sound ? "Mute sound" : "Enable sound"}
        >
          <svg viewBox="0 0 16 16" aria-hidden>
            <path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5z" fill="currentColor" />
            {sound ? (
              <>
                <path className="sound-wave w1" d="M10.5 5.5a3.5 3.5 0 010 5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
                <path className="sound-wave w2" d="M12.3 3.7a6 6 0 010 8.6" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
              </>
            ) : (
              <path className="sound-mute" d="M10.5 6l4 4M14.5 6l-4 4" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
            )}
          </svg>
          <span className="sound-label">{sound ? "Sound" : "Muted"}</span>
        </button>
      </div>
      <div className="scrubber">
        <div
          ref={trackRef}
          className="track"
          onPointerDown={(e) => {
            scrubbing.current = true
            e.currentTarget.setPointerCapture?.(e.pointerId)
            props.onPause()
            seekFromEvent(e)
          }}
          onPointerMove={(e) => scrubbing.current && seekFromEvent(e)}
          onPointerUp={() => (scrubbing.current = false)}
          onPointerCancel={() => (scrubbing.current = false)}
          onLostPointerCapture={() => (scrubbing.current = false)}
        >
          {BEATS.map((b, i) => {
            const end = BEATS[i + 1]?.t ?? T.end
            const fill = Math.min(1, Math.max(0, (t - b.t) / (end - b.t)))
            return (
              <div key={b.label} className="seg" style={{ flexGrow: end - b.t }}>
                <i style={{ transform: `scaleX(${fill})` }} />
              </div>
            )
          })}
        </div>
        <div className="beats">
          {BEATS.map((b, i) => {
            const end = BEATS[i + 1]?.t ?? T.end
            return (
              <button key={b.label} className={i === current ? "on" : ""} style={{ flexGrow: end - b.t }} onClick={() => props.onBeat(b.t)}>
                <span>{i + 1}</span>
                <em>{b.label}</em>
              </button>
            )
          })}
        </div>
      </div>
      <div className="controls-right">
        <button className={`draw-btn${drawMode ? " on" : ""}`} onClick={props.onDraw} title="Draw (or hold ⇧ and drag)">
          <kbd>⇧</kbd> Draw
        </button>
      </div>
    </div>
  )
}

const ReplayIcon = () => (
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden>
    <path d="M3 8a5 5 0 105-5H5.5" />
    <path d="M7 1L5 3l2 2" />
  </svg>
)

const MacIcon = () => (
  <svg viewBox="0 0 18 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" aria-hidden>
    <rect x="1.5" y="1.5" width="15" height="10" rx="2" />
    <path d="M6.5 14.5h5M9 11.5v3" strokeLinecap="round" />
  </svg>
)

const PhoneIcon = () => (
  <svg viewBox="0 0 12 18" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden>
    <rect x="1.5" y="1" width="9" height="16" rx="2.4" />
    <path d="M4.6 3.2h2.8" strokeLinecap="round" strokeWidth="1.6" />
  </svg>
)

function ModeSwitch({ mode, onMode }: { mode: Mode; onMode: (m: Mode) => void }) {
  return (
    <div className="mode-switch" role="tablist" aria-label="Capture source">
      <span className={`mode-thumb ${mode}`} aria-hidden />
      <button role="tab" aria-selected={mode === "mac"} className={mode === "mac" ? "on" : ""} onClick={() => onMode("mac")}>
        <MacIcon />
        Mac Screen
      </button>
      <button role="tab" aria-selected={mode === "iphone"} className={mode === "iphone" ? "on" : ""} onClick={() => onMode("iphone")}>
        <PhoneIcon />
        iPhone Mirror
      </button>
    </div>
  )
}

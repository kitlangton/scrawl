import { memo } from "react"
import {
  AGENT,
  AREA,
  clock,
  type Mode,
  PHONE,
  PILL,
  type Pill,
  pillState,
  pillWidth,
  progress,
  type Rect,
  recStart,
  revealEnd,
  type Script,
  STAGE,
  strokeAt,
  T,
  type Cursor as CursorState,
  MAC_WINDOW,
} from "./script"
import { INK_SHADOW, InkPath, PinMark } from "./ink"
import { BROKEN, type Fix, MacWindow, PhoneMirror } from "./apps"
import { easeOutCubic, motion, smoother, springAt } from "./motion"

const pressedAt = (t: number, at: number) => t >= at - 60 && t < at + 160

// Ghostty becomes the key window once the cursor clicks into the OpenCode prompt.
export const terminalActive = (t: number) => t >= T.promptClick

const APPLE =
  "M11.2 4.3c.6-.8 1-1.8.9-2.8-.9 0-2 .6-2.6 1.4-.6.7-1.1 1.7-.9 2.7 1 .1 2-.5 2.6-1.3zM12.1 5.8c-1.4-.1-2.7.8-3.3.8-.7 0-1.7-.8-2.9-.8C4.4 5.9 3 6.8 2.2 8.1c-1.6 2.7-.4 6.8 1.1 9 .8 1.1 1.7 2.3 2.8 2.3 1.1 0 1.6-.7 2.9-.7 1.4 0 1.8.7 2.9.7 1.2 0 2-1.1 2.7-2.2.9-1.3 1.2-2.5 1.2-2.6 0 0-2.4-.9-2.4-3.6 0-2.2 1.8-3.3 1.9-3.4-1-1.5-2.6-1.7-3.2-1.7z"

const MenuBarInner = memo(function MenuBarInner({ active }: { active: boolean }) {
  const app = active ? "Ghostty" : "Pulse"
  return (
    <div className="menubar">
      <div className="menubar-left">
        <svg className="apple" viewBox="0 0 17 21" aria-hidden>
          <path d={APPLE} />
        </svg>
        <b>{app}</b>
        <span>File</span>
        <span>Edit</span>
        <span>View</span>
        <span>Window</span>
        <span>Help</span>
      </div>
      <div className="menubar-right">
        <span className="menubar-extra menubar-scrawl">
          <img src="/menubar-icon@2x.png" width={16} height={16} decoding="async" alt="" />
        </span>
        <span className="menubar-extra">
          <span className="battery">
            <i />
          </span>
        </span>
        <span className="menubar-extra">
          <svg viewBox="0 0 20 14" className="wifi" aria-hidden>
            <path d="M10 13.2l2.3-2.6a3.4 3.4 0 00-4.6 0zM4.9 7.5l1.5 1.7a5.4 5.4 0 017.2 0l1.5-1.7a7.6 7.6 0 00-10.2 0zM1.6 4.1l1.5 1.7a10 10 0 0113.8 0l1.5-1.7a12.2 12.2 0 00-16.8 0z" />
          </svg>
        </span>
        <span className="menubar-extra">
          <svg viewBox="0 0 16 16" className="spot" aria-hidden>
            <circle cx="6.6" cy="6.6" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.7" />
            <path d="M10 10l4 4" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" />
          </svg>
        </span>
        <span className="menubar-extra">
          <svg viewBox="0 0 18 16" className="cc" aria-hidden>
            <rect x="1" y="1.5" width="16" height="5.6" rx="2.8" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <circle cx="4.4" cy="4.3" r="1.7" />
            <rect x="1" y="8.9" width="16" height="5.6" rx="2.8" />
            <circle cx="13.6" cy="11.7" r="1.7" fill="#2b2b33" />
          </svg>
        </span>
        <span className="menubar-clock">Tue Oct 13&nbsp;&nbsp;9:41 AM</span>
      </div>
    </div>
  )
})

export function MenuBar({ t }: { t: number }) {
  return <MenuBarInner active={terminalActive(t)} />
}

function Chip({
  w,
  children,
  on,
  primary,
  dim,
  pressed,
  round,
}: {
  w: number
  children: React.ReactNode
  on?: boolean
  primary?: boolean
  dim?: boolean
  pressed?: boolean
  round?: boolean
}) {
  const cls = ["chip", on && "on", primary && "primary", dim && "dim", pressed && "pressed", round && "round"].filter(Boolean).join(" ")
  return (
    <span className={cls} style={{ width: w }}>
      {children}
    </span>
  )
}

const Divider = () => <span className="island-divider" />

type IslandProps = {
  state: Pill
  phone: boolean
  // The iPhone chip was clicked (`phonePressed` while the button is down).
  phoneOn: boolean
  phonePressed: boolean
  recText: string
  // 1 while the switch chips show, 0 once they've folded into Cancel · Finish.
  chips: number
}

const islandItems = (state: Pill, phone: boolean, chips: number): number[] => {
  const reveal = phone ? PILL.items.revealPhone : PILL.items.reveal
  if (state === "notch" || state === "reveal") return reveal
  if (state === "recording") return chips > 0.5 ? reveal : PILL.items.recording
  return PILL.items[state]
}

const CloseChip = ({ w }: { w: number }) => (
  <Chip w={w} round>
    <svg viewBox="0 0 10 10" aria-hidden>
      <path d="M2 2l6 6M8 2L2 8" />
    </svg>
  </Chip>
)

// The recording capsule: the lead (dot + clock) stays pinned to the left edge while the trailing
// items swap in place, the switch chips fading out as Cancel · Finish fade in (src/ui.rs).
function RecordingContent({ phone, phoneOn, phonePressed, recText, chips }: Omit<IslandProps, "state">) {
  const reveal = phone ? PILL.items.revealPhone : PILL.items.reveal
  const [lead] = reveal
  const [, , cancel, finish] = PILL.items.recording
  const blur = (k: number) => (k > 0.001 && k < 0.999 ? `blur(${(Math.sin(Math.PI * k) * 2).toFixed(2)}px)` : undefined)
  return (
    <>
      <span className="island-lead" style={{ width: lead }}>
        <span className="rec-slot">
          <i className="rec-halo" />
          <i className="rec-dot" />
        </span>
        <b>{recText}</b>
      </span>
      <Divider />
      <span className="island-trail">
        {chips > 0.001 && (
          <span className="island-chips" style={{ opacity: chips, filter: blur(chips) }}>
            <Chip w={reveal[2]}>Area</Chip>
            <Chip w={reveal[3]}>Window</Chip>
            <Chip w={reveal[4]} on={!phoneOn}>
              Screen
            </Chip>
            {phone && (
              <Chip w={reveal[5]} on={phoneOn} pressed={phonePressed}>
                iPhone
              </Chip>
            )}
            <Divider />
            <CloseChip w={reveal[reveal.length - 1]} />
          </span>
        )}
        {chips < 0.999 && (
          <span className="island-controls" style={{ opacity: 1 - chips, filter: blur(chips) }}>
            <Chip w={cancel}>Cancel</Chip>
            <Chip w={finish} primary>
              Finish
            </Chip>
          </span>
        )}
      </span>
    </>
  )
}

function IslandContent({ state, ...props }: IslandProps) {
  const items = islandItems(state, props.phone, props.chips)
  const [lead, , c2, c3, c4] = items
  return (
    <>
      {(state === "reveal" || state === "recording") && <RecordingContent {...props} />}
      {state === "processing" && (
        <span className="island-lead" style={{ width: lead }}>
          <span className="orbit">
            {Array.from({ length: 8 }, (_, i) => (
              <i key={i} style={{ transform: `rotate(${i * 45}deg) translateY(-7.2px)`, animationDelay: `${(i / 8) * 0.476 - 0.476}s` }} />
            ))}
          </span>
          <b>Processing…</b>
        </span>
      )}
      {state === "copied" && (
        <>
          <span className="island-lead" style={{ width: lead }}>
            <span className="check">
              <svg viewBox="0 0 12 12" aria-hidden>
                <path d="M2.6 6.3l2.2 2.2 4.6-4.9" />
              </svg>
            </span>
            <b>Copied</b>
          </span>
          <Divider />
          <Chip w={c2} primary>
            Paste
          </Chip>
          <Chip w={c3}>Inspect</Chip>
          <CloseChip w={c4} />
        </>
      )}
    </>
  )
}

type Leaving = { state: Pill; opacity: number } | null

const IslandInner = memo(function IslandInner({ gone, leaving, ...props }: IslandProps & { gone: boolean; leaving: Leaving }) {
  const { state } = props
  const width = pillWidth(islandItems(state, props.phone, props.chips))
  const recording = state === "reveal" || state === "recording"
  return (
    <div
      className={`island ${state}${recording ? " pinned" : ""}${gone ? " gone" : ""}`}
      style={{ left: PILL.cx, top: PILL.top, width, height: PILL.h }}
      aria-hidden={gone}
    >
      {leaving && (
        <div className="island-content island-leaving" style={{ opacity: leaving.opacity }} aria-hidden>
          <IslandContent {...props} state={leaving.state} chips={0} phoneOn={false} phonePressed={false} />
        </div>
      )}
      {/* One key across reveal and recording, so the lead never remounts when the chips fold. */}
      <div className="island-content" key={recording ? "recording" : state}>
        <IslandContent {...props} />
      </div>
    </div>
  )
})

// The HUD's content cross-fades when its state changes: the old chips fade out in place over
// 140 ms while the new ones fade in, instead of vanishing in one frame.
const ISLAND_SWAPS: [number, Pill][] = [
  [T.finish + 50, "recording"],
  [T.copied, "processing"],
]

const FOLD_MS = 160

export function Island({ t, mode }: { t: number; mode: Mode }) {
  const state = pillState(t, mode)
  const phone = mode === "iphone"
  const fold = revealEnd(mode)
  const chips = t < fold ? 1 : +(1 - smoother((t - fold) / FOLD_MS)).toFixed(3)
  const gone = state === "notch" || t >= T.paste
  const swap = ISLAND_SWAPS.find(([at]) => t >= at && t < at + 140)
  const leaving = swap ? { state: swap[1], opacity: +(1 - smoother((t - swap[0]) / 140)).toFixed(3) } : null
  const recording = state === "reveal" || state === "recording"
  return (
    <IslandInner
      state={state}
      phone={phone}
      phoneOn={phone && t >= T.phoneClick - 60}
      phonePressed={phone && pressedAt(t, T.phoneClick)}
      recText={recording ? clock(t - recStart(mode, t)) : clock(T.finish - recStart(mode, t))}
      chips={state === "notch" ? 1 : chips}
      gone={gone}
      leaving={leaving}
    />
  )
}

// A tap-started Screen recording: a soft red glow along the display's edges rises in 0.12 s and is
// gone by 1.1 s (laser_overlay.metal), so the start shows without the picker.
export function EdgeGlow({ t }: { t: number }) {
  const s = (t - T.keysUp) / 1000
  if (s <= 0 || s >= 1.1) return null
  const smoothstep = (a: number, b: number, x: number) => {
    const k = Math.min(1, Math.max(0, (x - a) / (b - a)))
    return k * k * (3 - 2 * k)
  }
  const env = smoothstep(0, 0.12, s) * (1 - smoothstep(0.35, 1.1, s))
  return <div className="edge-glow" style={{ opacity: +env.toFixed(3) }} aria-hidden />
}

export function Ink({ t, script }: { t: number; script: Script }) {
  if (t < T.circle[0]) return null
  const fade = 1 - progress(t, T.finish, T.finish + 280)
  if (fade <= 0) return null
  const m = motion()
  return (
    <svg className="ink" viewBox={`0 0 ${STAGE.w} ${STAGE.h}`} style={{ opacity: fade, ...INK_SHADOW }}>
      {script.strokes.map((s) => {
        const live = strokeAt(s, t)
        return live ? <InkPath key={s.id} d={live.d} /> : null
      })}
      {script.pins.map((p) => {
        if (t < p.t) return null
        const age = t - p.t
        const scale = age > 1200 ? 1 : Math.max(0, springAt(age, m.pin))
        return <PinMark key={p.n} n={p.n} at={p.at} scale={scale} />
      })}
    </svg>
  )
}

export function Ripples({ t, script }: { t: number; script: Script }) {
  const { rippleMs, rippleAlpha } = motion()
  const active = script.clicks.filter((c) => t > c.t && t < c.t + rippleMs)
  if (!active.length) return null
  return (
    <svg className="ink" viewBox={`0 0 ${STAGE.w} ${STAGE.h}`}>
      {active.map((c) => {
        const p = progress(t, c.t, c.t + rippleMs)
        // A quick ring that opens fast and fades on a convex tail.
        const fade = (1 - p) * (1 - p)
        return <circle key={c.t} cx={c.at[0]} cy={c.at[1]} r={6 + 16 * easeOutCubic(p)} fill="none" stroke="#fff" strokeWidth={1.5} opacity={rippleAlpha * fade} />
      })}
    </svg>
  )
}

export function Cursor({ c, ibeam }: { c: CursorState; ibeam?: boolean }) {
  const [x, y] = c.at
  return (
    <div className="cursor" style={{ transform: `translate3d(${x.toFixed(2)}px, ${y.toFixed(2)}px, 0)` }}>
      {c.shift ? (
        <svg className="cursor-glyph" width="26" height="26" viewBox="0 0 26 26" style={{ transform: `translate(-2px, -24px) scale(${c.pressed ? 0.94 : 1})` }}>
          <path d="M3 23l2-7L18 3l5 5L10 21z" fill="#f2f2f2" stroke="#111" strokeWidth="1.3" strokeLinejoin="round" />
          <path d="M3 23l2-7 5 5z" fill="#ff3b30" stroke="#111" strokeWidth="1.3" strokeLinejoin="round" />
          <path d="M15.5 5.5l5 5" stroke="#111" strokeWidth="1.3" />
        </svg>
      ) : ibeam ? (
        <svg width="10" height="20" viewBox="0 0 10 20" style={{ transform: "translate(-5px, -10px)" }}>
          <path d="M1.5 1.5c2 0 3.5.6 3.5 2v13c0 1.4-1.5 2-3.5 2M8.5 1.5c-2 0-3.5.6-3.5 2v13c0 1.4 1.5 2 3.5 2M3 10h4" fill="none" stroke="#fff" strokeWidth="3" strokeLinecap="round" />
          <path d="M1.5 1.5c2 0 3.5.6 3.5 2v13c0 1.4-1.5 2-3.5 2M8.5 1.5c-2 0-3.5.6-3.5 2v13c0 1.4 1.5 2 3.5 2M3 10h4" fill="none" stroke="#000" strokeWidth="1.1" strokeLinecap="round" />
        </svg>
      ) : (
        <svg className="cursor-glyph" width="22" height="28" viewBox="0 0 22 28" style={{ transform: `translate(-2px, -2px) scale(${c.pressed ? 0.88 : 1})`, transformOrigin: "2px 2px" }}>
          <path d="M2 2v20.5l5.2-5 3.4 8 3.6-1.5-3.4-7.8h7.3z" fill="#000" stroke="#fff" strokeWidth="1.6" strokeLinejoin="round" />
        </svg>
      )}
      {c.shift && <span className="cursor-key">⇧</span>}
    </div>
  )
}

const KEYS = ["⌥", "⇧", "S"]

// The two taps of ⌥⇧S: one starts recording as the keys come up, the next finishes as S goes down.
const TAPS = [
  { down: T.keys, up: T.keysUp },
  { down: T.finish - 160, up: T.finish + 120 },
]

export function Keys({ t, mode }: { t: number; mode: Mode }) {
  const cx = mode === "mac" ? MAC_WINDOW.x + MAC_WINDOW.w / 2 : PHONE.x + PHONE.w / 2
  const tap = TAPS.find((k) => t >= k.down - 150 && t < k.up + 350)
  const paste = t >= T.paste - 400 && t < T.paste + 500
  if (tap) {
    const out = progress(t, tap.up, tap.up + 350)
    return (
      <div className="keys" style={{ left: cx, top: 646, opacity: (1 - out) * Math.min(1, progress(t, tap.down - 150, tap.down)) }}>
        {KEYS.map((k, i) => (
          <kbd key={k} className={t >= tap.down + i * 80 && t < tap.up ? "down" : ""}>
            {k}
          </kbd>
        ))}
      </div>
    )
  }
  if (paste) {
    const out = progress(t, T.paste + 250, T.paste + 500)
    return (
      <div className="keys small" style={{ left: AGENT.x + AGENT.w / 2, top: AGENT.y + AGENT.h - 150, opacity: (1 - out) * Math.min(1, progress(t, T.paste - 400, T.paste - 250)) }}>
        <kbd className={t >= T.paste - 300 && t < T.paste + 250 ? "down" : ""}>⌥</kbd>
        <kbd className={t >= T.paste - 150 && t < T.paste + 250 ? "down" : ""}>⇧</kbd>
        <kbd className={t >= T.paste && t < T.paste + 200 ? "down" : ""}>V</kbd>
      </div>
    )
  }
  return null
}

// The spoken words follow the pen: each clause's pill sits beside the mark it describes.
// Both clauses occupy the same CSS grid cell so the pill size stays constant and never reflows.
const ramp = (t: number, a: number, b: number) => (t <= a ? 0 : t >= b ? 1 : (t - a) / (b - a))

export function Caption({ t, script }: { t: number; script: Script }) {
  const { words: all, clause2 } = script.narration
  const first = all[0][1]
  if (t < first - 300 || t > T.finish + 400) return null
  const m = motion()
  const swap = all[clause2][1] - 60
  const c1Words = all.slice(0, clause2)
  const c2Words = all.slice(clause2)

  const envIn = easeOutCubic(progress(t, first - 280, first - 20))
  const envOut = 1 - easeOutCubic(progress(t, T.finish, T.finish + 320))
  const opacity = Math.min(envIn, envOut)
  if (opacity <= 0.005) return null

  // The pill travels to the second mark on a minimum-jerk curve while the clauses cross.
  const glide = smoother(progress(t, swap - 240, swap + 300))
  const [a, b] = script.speech
  const x = a[0] + (b[0] - a[0]) * glide
  const y = a[1] + (b[1] - a[1]) * glide + (1 - envIn) * 4

  const D = m.clauseMs
  const c1Out = easeOutCubic(progress(t, swap - 0.6 * D, swap + 0.1 * D))
  const c2In = easeOutCubic(progress(t, swap - 0.25 * D, swap + 0.75 * D))
  // Loudness envelope for the wave: eases up into speech and back down after it.
  let talk = 0
  for (const [, at, end] of all) talk = Math.max(talk, Math.min(ramp(t, at - 90, at + 30), 1 - ramp(t, end + 60, end + 260)))

  const renderWord = ([w, at, end]: [string, number, number], i: number) => {
    const dt = t - (at - 35)
    const text = w.replace("—", "\u2060—\u2060")
    if (dt <= 0) {
      return (
        <span key={i} className="w-slot">
          <span className="w pending">{text}</span>{" "}
        </span>
      )
    }
    const appear = easeOutCubic(dt / m.wordMs)
    const rise = (1 - appear) * m.wordRise
    const blur = (1 - appear) * m.wordBlur
    // The word being spoken sits at full white; spoken words settle back to 68%.
    const now = Math.min(ramp(t, at - 80, at), 1 - ramp(t, end + 120, end + 340))
    return (
      <span key={i} className="w-slot">
        <span
          className="w"
          style={{
            opacity: appear,
            color: `rgba(255, 255, 255, ${(0.68 + 0.32 * now).toFixed(3)})`,
            transform: rise > 0.02 ? `translate3d(0, ${rise.toFixed(2)}px, 0)` : undefined,
            filter: blur > 0.05 ? `blur(${blur.toFixed(2)}px)` : undefined,
          }}
        >
          {text}
        </span>{" "}
      </span>
    )
  }

  const clauseStyle = (shown: number, dir: number): React.CSSProperties => ({
    opacity: shown,
    transform: shown < 0.999 ? `translate3d(0, ${(dir * 3 * (1 - shown)).toFixed(2)}px, 0)` : undefined,
    filter: shown > 0.001 && shown < 0.999 && m.wordBlur > 0 ? `blur(${(m.wordBlur * (1 - shown)).toFixed(2)}px)` : undefined,
  })

  return (
    <div
      className="caption"
      style={{
        transform: `translate3d(${x.toFixed(2)}px, ${y.toFixed(2)}px, 0) translate(-100%, -50%)`,
        opacity,
      }}
    >
      <span className="wave">
        {[0, 1, 2, 3, 4].map((i) => {
          const amp = 0.16 + talk * (0.19 + 0.65 * Math.abs(Math.sin(t / (90 + i * 23) + i * 1.7)))
          return <i key={i} style={{ transform: `scaleY(${amp.toFixed(3)})` }} />
        })}
      </span>
      <div className="caption-clauses">
        <p className="caption-clause" style={clauseStyle(1 - c1Out, -1)} aria-hidden={c1Out >= 0.99}>
          {c1Words.map(renderWord)}
        </p>
        <p className="caption-clause" style={clauseStyle(c2In, 1)} aria-hidden={c2In <= 0.01}>
          {c2Words.map(renderWord)}
        </p>
      </div>
    </div>
  )
}

export const captureArea = (mode: Mode): Rect => (mode === "mac" ? AREA : { x: PHONE.x - 6, y: PHONE.y - 6, w: PHONE.w + 12, h: PHONE.h + 12 })

export const KeyframeScene = memo(function KeyframeScene({
  mode,
  script,
  frame,
  scale,
}: {
  mode: Mode
  script: Script
  frame: Script["frames"][number]
  scale: number
}) {
  const area = captureArea(mode)
  return (
    <div className="sheet-scene wallpaper" style={{ width: STAGE.w, height: STAGE.h, transform: `scale(${scale}) translate(${-area.x}px, ${-area.y}px)` }}>
      {mode === "mac" ? <MacWindow fix={BROKEN} /> : <PhoneMirror fix={BROKEN} />}
      <svg className="ink" viewBox={`0 0 ${STAGE.w} ${STAGE.h}`} style={INK_SHADOW}>
        {script.strokes
          .filter((st) => frame.strokes.includes(st.id))
          .map((st) => (
            <InkPath key={st.id} d={st.sheetPath} />
          ))}
        {script.pins
          .filter((p) => frame.pins.includes(p.n))
          .map((p) => (
            <PinMark key={p.n} n={p.n} at={p.at} scale={1.7} />
          ))}
      </svg>
    </div>
  )
})

// Corner index badge drawn by render.rs: a 22px dark pill with a faint ring, top-right.
export function FrameBadge({ n, k }: { n: number; k: number }) {
  const s = Math.max(k, 0.42)
  return (
    <span className="frame-badge" style={{ height: 22 * s, minWidth: 22 * s, padding: `0 ${6.5 * s}px`, fontSize: 12 * s, top: 6 * s, right: 6 * s }}>
      {n}
    </span>
  )
}

// Same geometry as render_contact_sheet: 640px cells (440 for portrait), 16px padding, 14px gaps,
// 10px card radius with a 1px #2e2e30 rim on a #141414 sheet.
export const ContactSheet = memo(function ContactSheet({
  mode,
  script,
  width,
  height,
  develop,
}: {
  mode: Mode
  script: Script
  width: number
  height?: number
  develop?: boolean
}) {
  const area = captureArea(mode)
  const portrait = area.h > area.w
  const cell = portrait ? 440 : 640
  const fw0 = Math.min(cell, (960 * area.w) / area.h)
  const fh0 = (fw0 * area.h) / area.w
  const n = script.frames.length
  const w0 = 32 + cell * n + 14 * (n - 1)
  const h0 = 32 + fh0
  const k = Math.min(width / w0, (height ?? Infinity) / h0)
  return (
    <div className="sheet" style={{ width: w0 * k, height: h0 * k, padding: 16 * k, gap: 14 * k, borderRadius: Math.max(2, 6 * k) }}>
      {script.frames.map((f, idx) => (
        <div key={f.n} className="sheet-frame" style={{ width: cell * k, height: fh0 * k }}>
          <div
            className={`sheet-card${develop ? " develop" : ""}`}
            style={{ width: fw0 * k, height: fh0 * k, borderRadius: 10 * k, animationDelay: develop ? `${idx * 85}ms` : undefined }}
          >
            <KeyframeScene mode={mode} script={script} frame={f} scale={(fw0 * k) / area.w} />
            <FrameBadge n={f.n} k={k} />
          </div>
        </div>
      ))}
    </div>
  )
})

// The fix lands in diff order: the badge first, then the rows and the button row together. Those two
// share a start because the rows push the buttons down while the buttons rise out of their clip; any
// offset between them makes the button row bob down, up and down again. A spring within `eps` of rest snaps to exactly 1: the rows use a coarser eps
// because their padding is layout, and a long sub-pixel creep only shows as rounding shimmer.
const settled = (ms: number, s: { duration: number; bounce: number }, eps = 0.001) => {
  const v = springAt(ms, s)
  return ms > 250 && Math.abs(1 - v) < eps ? 1 : v
}

// The sheen starts as each element locks in, about 40% of its spring's duration after it starts.
const sheenAt = (ms: number, s: { duration: number }, sheenMs: number) => {
  const p = (ms - s.duration * 400) / sheenMs
  return p < 0 || p > 1 ? -1 : (p + smoother(p)) / 2
}

export function fixState(t: number): Fix {
  if (t < T.fix[0]) return BROKEN
  const m = motion()
  const ms = t - T.fix[0]
  return {
    badge: settled(ms, m.badge),
    rows: settled(ms - m.stagger, m.rows, 0.01),
    actions: settled(ms - m.stagger, m.actions),
    badgeSheen: sheenAt(ms, m.badge, m.sheenMs),
    actionsSheen: sheenAt(ms - m.stagger, m.actions, m.sheenMs),
  }
}

// When each diff hunk lands on the stage (ms after T.fix[0]): the badge, then rows and buttons.
export const hunkDelay = (hunk: number) => (hunk === 0 ? 0 : motion().stagger)

export const phoneLive = (t: number) => Math.min(progress(t, T.phoneRecord, T.phoneRecord + 300), 1 - progress(t, T.finish, T.finish + 300))

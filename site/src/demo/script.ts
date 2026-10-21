import { inkPath } from "./ink"

export type Mode = "mac" | "iphone"
export type Point = [number, number]
export type Rect = { x: number; y: number; w: number; h: number }

export const STAGE = { w: 1200, h: 720 }
export const INK = "#ff3b30"

export const MENUBAR_H = 28
export const MAC_WINDOW: Rect = { x: 52, y: 78, w: 684, h: 540 }
export const PHONE: Rect = { x: 265, y: 76, w: 258, h: 556 }
export const AGENT: Rect = { x: 760, y: 48, w: 422, h: 640 }
export const AREA: Rect = { x: 40, y: 66, w: 708, h: 564 }

// A tap of ⌥⇧S records the screen as the keys come up (`keysUp`), with a red edge glow, and the
// capsule shows the switch chips (Area · Window · Screen · iPhone) until `REVEAL_MS` pass or the
// first stroke. On iPhone the iPhone chip restarts the recording on the phone (`phoneRecord`).
// The narration and the marks are timed from `record`; another tap finishes at `finish`.
export const T = {
  keys: 2000,
  keysUp: 2300,
  phoneClick: 3450,
  phoneRecord: 3600,
  record: 3600,
  circle: [5600, 7100] as const,
  pin1: 7600,
  arrow: [9200, 9900] as const,
  head: [10000, 10350] as const,
  pin2: 11000,
  finish: 13900,
  copied: 14600,
  promptClick: 15000,
  paste: 15200,
  submit: 15700,
  read: 16300,
  edit: 16800,
  diff: 17000,
  fix: [18300, 19300] as const,
  done: 19700,
  end: 21300,
}

// `crate::shortcut::REVEAL_MS`.
export const REVEAL_MS = 3500

// When the recording the capsule's clock counts began: the screen on the Mac, the phone on iPhone.
export const recStart = (mode: Mode, t = Infinity) => (mode === "iphone" && t >= T.phoneRecord ? T.phoneRecord : T.keysUp)

// When the switch chips fold into Cancel · Finish.
export const revealEnd = (mode: Mode) => (mode === "iphone" ? T.phoneRecord : Math.min(T.keysUp + REVEAL_MS, T.circle[0]))

export const BEATS = [
  { label: "Broken", t: 0 },
  { label: "Capture", t: 1800 },
  { label: "Annotate", t: T.record },
  { label: "Handoff", t: 13700 },
  { label: "Fix", t: 16600 },
] as const

// Word onsets in ms from the start of each narration file (measured from the ElevenLabs alignment).
const MAC_WORDS: [string, number][] = [
  ["Look", 1200],
  ["at", 1360],
  ["this", 1560],
  ["header", 1840],
  ["badge…", 2080],
  ["it's", 3120],
  ["colliding", 3360],
  ["with", 3880],
  ["the", 4000],
  ["title,", 4160],
  ["and", 5120],
  ["this", 5320],
  ["button", 5600],
  ["row", 5840],
  ["is", 6080],
  ["shifted", 6240],
  ["down", 6640],
  ["14", 7040],
  ["pixels.", 7920],
  ["Line", 8880],
  ["them", 9120],
  ["up.", 9360],
]

const IPHONE_WORDS: [string, number][] = [
  ["On", 1124],
  ["the", 1324],
  ["phone", 1524],
  ["it's", 1804],
  ["even", 2004],
  ["worse…", 2324],
  ["this", 3324],
  ["live", 3524],
  ["pill", 3844],
  ["is", 4124],
  ["right", 4324],
  ["on", 4564],
  ["top", 4764],
  ["of", 4924],
  ["the", 5044],
  ["header,", 5284],
  ["and", 5994],
  ["the", 6084],
  ["checkout", 6364],
  ["bar", 6644],
  ["is", 6804],
  ["cut", 7044],
  ["off", 7204],
  ["at", 7324],
  ["the", 7444],
  ["bottom.", 7724],
  ["Please", 8564],
  ["fix", 8884],
  ["it.", 9204],
]

export type Narration = {
  src: string
  // [word, start, end] on the demo clock.
  words: [string, number, number][]
  // Index of the word that opens the second clause.
  clause2: number
  // Demo-clock time when the file ends.
  end: number
  // Seconds into the file to hold silent: the gaps where the voice model's direction tags sit.
  mute: [number, number][]
}

// Both recordings start with a sigh at the moment recording begins.
const narration = (src: string, spoken: [string, number][], speechEnd: number, duration: number, mute: [number, number][]): Narration => {
  const words = spoken.map(([w, at], i): [string, number, number] => {
    const next = spoken[i + 1]?.[1] ?? speechEnd
    return [w, T.record + at, T.record + Math.min(next - 40, at + 520)]
  })
  return { src, words, clause2: spoken.findIndex(([w]) => w === "and"), end: T.record + duration, mute }
}

// The mute windows stop short of the next word's onset (measured RMS rises ~60 ms before it).
export const NARRATION: Record<Mode, Narration> = {
  mac: narration("/narration.mp3", MAC_WORDS, 9900, 9840, [
    [0.7, 1.05],
    [4.8, 5.03],
  ]),
  iphone: narration("/narration-iphone.mp3", IPHONE_WORDS, 9750, 9680, [
    [0.82, 1.04],
    [5.7, 5.86],
  ]),
}

export const WORDS = NARRATION.mac.words

export const clamp = (v: number, lo = 0, hi = 1) => Math.min(hi, Math.max(lo, v))
export const lerp = (a: number, b: number, p: number) => a + (b - a) * p
export const progress = (t: number, a: number, b: number) => clamp((t - a) / (b - a))
export const easeInOut = (p: number) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2)
export const easeOut = (p: number) => 1 - Math.pow(1 - p, 3)
export const easeOutBack = (p: number) => {
  const c1 = 1.4
  const c3 = c1 + 1
  return 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2)
}

export const clock = (ms: number) => {
  const s = Math.max(0, Math.floor(ms / 1000))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`
}

export type Pill = "notch" | "reveal" | "recording" | "processing" | "copied"

export const pillState = (t: number, mode: Mode): Pill => {
  if (t < T.keysUp) return "notch"
  if (t < revealEnd(mode)) return "reveal"
  if (t < T.finish + 50) return "recording"
  if (t < T.copied) return "processing"
  return "copied"
}

// Mirrors the HUD in src/ui.rs: a 40px capsule, 4px inset, 32px chips with 2px gaps. Item widths
// are fixed (a divider is 1px plus 6px margins) so the scripted cursor can hit exact chip centers.
export const PILL = {
  cx: STAGE.w / 2,
  top: MENUBAR_H + 10,
  h: 40,
  inset: 4,
  gap: 2,
  items: {
    // Lead (dot + clock), divider, Area · Window · Screen, divider, ×.
    reveal: [70, 13, 56, 72, 66, 13, 32],
    // With a cabled iPhone, its chip after Screen.
    revealPhone: [70, 13, 56, 72, 66, 66, 13, 32],
    recording: [70, 13, 68, 68],
    processing: [132],
    copied: [98, 13, 62, 74, 32],
  },
}
export const PILL_CY = PILL.top + PILL.h / 2

export const pillWidth = (items: number[]) => items.reduce((a, b) => a + b, 0) + PILL.gap * (items.length - 1) + PILL.inset * 2

const chipCenter = (items: number[], index: number): Point => {
  let x = PILL.cx - pillWidth(items) / 2 + PILL.inset
  for (let i = 0; i < index; i++) x += items[i] + PILL.gap
  return [x + items[index] / 2, PILL_CY]
}

// The home-screen prompt row inside the OpenCode terminal (see Terminal in stage-parts.tsx).
export const PROMPT_AT: Point = [AGENT.x + 150, AGENT.y + 356]

export type Anchors = { badge: Rect; actions: Rect; title: Rect }

export type Stroke = { id: string; points: Point[]; t0: number; t1: number; fullPath: string; sheetPath: string }
export type Pin = { n: number; t: number; at: Point }
export type Click = { t: number; at: Point }
export type Waypoint = { t: number; at: Point; arc?: number }

export type Script = {
  strokes: Stroke[]
  // Where the spoken caption pill's right edge sits (vertically centered) for each clause.
  speech: [Point, Point]
  pins: Pin[]
  clicks: Click[]
  path: Waypoint[]
  frames: { n: number; t: number; strokes: string[]; pins: number[] }[]
  narration: Narration
}

const circlePoints = (cx: number, cy: number, rx: number, ry: number): Point[] => {
  const pts: Point[] = []
  const start = -2.5
  const sweep = Math.PI * 2 * 1.1
  const n = 96
  for (let i = 0; i <= n; i++) {
    const k = i / n
    const a = start + sweep * k
    // A slow outward drift lets the overlap land beside the start instead of on top of it.
    const grow = 1 + k * 0.06
    pts.push([cx + Math.cos(a) * rx * grow, cy + Math.sin(a) * ry * grow - k * 4])
  }
  return pts
}

const curvePoints = (s: Point, c: Point, e: Point, n = 36): Point[] => {
  const pts: Point[] = []
  for (let i = 0; i <= n; i++) {
    const k = i / n
    const u = 1 - k
    pts.push([u * u * s[0] + 2 * u * k * c[0] + k * k * e[0], u * u * s[1] + 2 * u * k * c[1] + k * k * e[1]])
  }
  return pts
}

const line = (a: Point, b: Point, n = 10): Point[] =>
  Array.from({ length: n + 1 }, (_, i) => [lerp(a[0], b[0], i / n), lerp(a[1], b[1], i / n)] as Point)

const center = (r: Rect): Point => [r.x + r.w / 2, r.y + r.h / 2]

const makeStroke = (id: string, points: Point[], t0: number, t1: number): Stroke => ({
  id,
  points,
  t0,
  t1,
  fullPath: inkPath(points, 4.5, 0),
  sheetPath: inkPath(points, 10, 0),
})

export function buildScript(mode: Mode, a: Anchors): Script {
  const badge = center(a.badge)
  const circle = circlePoints(badge[0] - 10, badge[1], a.badge.w / 2 + 40, a.badge.h / 2 + 15)

  // On the Mac the arrow swings up from below-left into the clipped "Retry failed" button.
  const buttons = a.actions.x + a.actions.w - 200
  const tip: Point =
    mode === "mac"
      ? [buttons - 10, a.actions.y + a.actions.h * 0.6]
      : [a.actions.x + a.actions.w * 0.62, a.actions.y + a.actions.h + 6]
  const start: Point = mode === "mac" ? [tip[0] - 104, tip[1] + 52] : [tip[0] + 34, tip[1] + 92]
  const ctrl: Point = mode === "mac" ? [tip[0] - 44, tip[1] + 46] : [tip[0] + 44, tip[1] + 34]
  const shaft = curvePoints(start, ctrl, tip, 40)
  // A quadratic's end tangent points from the control point to the tip.
  const angle = Math.atan2(tip[1] - ctrl[1], tip[0] - ctrl[0])
  const wing = (side: number): Point => [tip[0] - Math.cos(angle + side * 0.5) * 19, tip[1] - Math.sin(angle + side * 0.5) * 19]
  const wing1 = wing(1)
  const wing2 = wing(-1)
  const head1 = line(tip, wing1, 12)
  const head2 = line(tip, wing2, 12)
  const headMid = Math.round((T.head[0] + T.head[1]) / 2)

  const strokes: Stroke[] = [
    makeStroke("circle", circle, T.circle[0], T.circle[1]),
    makeStroke("shaft", shaft, T.arrow[0], T.arrow[1]),
    makeStroke("wing1", head1, T.head[0], headMid - 15),
    makeStroke("wing2", head2, headMid + 15, T.head[1]),
  ]

  // The pill rides to the left of each mark, so it never covers the ink or the thing being
  // described, and its glide between clauses never crosses the circle.
  const circleLeft = Math.min(...circle.map((p) => p[0]))
  const speech: Script["speech"] =
    mode === "mac"
      ? [
          [circleLeft - 12, badge[1]],
          [start[0] - 20, start[1]],
        ]
      : [
          [PHONE.x - 16, badge[1]],
          [PHONE.x - 16, start[1]],
        ]

  const pin1: Point = [a.badge.x + a.badge.w + 2, a.badge.y - 2]
  const pin2: Point =
    mode === "mac" ? [a.actions.x + a.actions.w - 92, a.actions.y + 10] : [a.actions.x + a.actions.w * 0.52, a.actions.y + 8]
  const pins: Pin[] = [
    { n: 1, t: T.pin1, at: pin1 },
    { n: 2, t: T.pin2, at: pin2 },
  ]

  const prompt: Point = PROMPT_AT
  const clicks: Click[] = [...pins.map((p) => ({ t: p.t, at: p.at })), { t: T.promptClick, at: prompt }]

  const path: Waypoint[] = [
    { t: 0, at: [840, 540] },
    { t: 320, at: [840, 540], arc: -0.07 },
    { t: 1650, at: [520, 300], arc: 0 },
  ]

  if (mode === "mac") {
    path.push({ t: T.circle[0] - 1400, at: [520, 300], arc: 0.06 })
  } else {
    const phoneChip = chipCenter(PILL.items.revealPhone, 5)
    clicks.push({ t: T.phoneClick, at: phoneChip })
    path.push(
      { t: T.keysUp + 200, at: [520, 300], arc: 0.06 },
      { t: T.phoneClick - 80, at: phoneChip, arc: 0 },
      { t: T.phoneClick + 160, at: phoneChip, arc: 0 },
      { t: T.circle[0] - 1400, at: phoneChip, arc: -0.07 },
    )
  }

  const restAfterPin2: Point = [pin2[0] + 28, pin2[1] + 34]

  path.push(
    { t: T.circle[0] - 80, at: circle[0], arc: 0 },
    { t: T.circle[0], at: circle[0], arc: 0 },
    { t: T.circle[1], at: circle[circle.length - 1], arc: 0.05 },
    { t: T.pin1 - 70, at: pin1, arc: 0 },
    { t: T.pin1 + 160, at: pin1, arc: -0.06 },
    { t: T.arrow[0] - 80, at: shaft[0], arc: 0 },
    { t: T.arrow[0], at: shaft[0], arc: 0 },
    { t: T.arrow[1], at: tip, arc: 0 },
    { t: T.head[0], at: tip, arc: 0 },
    { t: headMid - 15, at: wing1, arc: 0 },
    { t: headMid + 15, at: tip, arc: 0 },
    { t: T.head[1], at: wing2, arc: 0.05 },
    { t: T.pin2 - 70, at: pin2, arc: 0 },
    { t: T.pin2 + 160, at: pin2, arc: 0.04 },
    { t: T.pin2 + 650, at: restAfterPin2, arc: 0 },
    { t: T.finish + 170, at: restAfterPin2, arc: 0.06 },
    { t: T.promptClick - 90, at: prompt, arc: 0 },
    { t: T.promptClick + 130, at: prompt, arc: 0.06 },
    { t: T.paste + 680, at: [AGENT.x - 18, AGENT.y + AGENT.h - 10], arc: 0 },
  )

  return {
    strokes,
    speech,
    pins,
    clicks,
    path,
    narration: NARRATION[mode],
    frames: [
      { n: 1, t: T.pin1 - T.record, strokes: ["circle"], pins: [1] },
      { n: 2, t: T.pin2 - T.record, strokes: ["shaft", "wing1", "wing2"], pins: [2] },
    ],
  }
}

// The tip is interpolated between samples so the stroke grows continuously every frame.
export function strokeAt(s: Stroke, t: number) {
  if (t <= s.t0) return null
  if (t >= s.t1) return { points: s.points, tip: s.points[s.points.length - 1], done: true, d: s.fullPath }
  const p = easeInOut(progress(t, s.t0, s.t1))
  const last = s.points.length - 1
  const at = p * last
  const i = Math.floor(at)
  if (i >= last) return { points: s.points, tip: s.points[last], done: true, d: s.fullPath }
  const frac = at - i
  const a = s.points[i]
  const b = s.points[i + 1]
  const tip: Point = [lerp(a[0], b[0], frac), lerp(a[1], b[1], frac)]
  const head = i > 0 && frac < 0.2 ? s.points.slice(0, i) : s.points.slice(0, i + 1)
  const pts = [...head, tip]
  return { points: pts, tip, done: false, d: inkPath(pts, 4.5, 0) }
}

export type Cursor = { at: Point; drawing: boolean; pressed: boolean; shift: boolean }

export function cursorAt(script: Script, t: number): Cursor {
  const active = script.strokes.find((s) => t > s.t0 && t < s.t1)
  const pressed = script.clicks.some((c) => t >= c.t - 60 && t < c.t + 140)
  const shift = !!active || script.pins.some((p) => t >= p.t - 250 && t < p.t + 250)
  if (active) {
    return { at: strokeAt(active, t)!.tip, drawing: true, pressed: true, shift: true }
  }
  const path = script.path
  if (t <= path[0].t) return { at: path[0].at, drawing: false, pressed, shift }
  for (let i = 1; i < path.length; i++) {
    const a = path[i - 1]
    const b = path[i]
    if (t <= b.t) {
      const k = easeInOut(progress(t, a.t, b.t))
      const dx = b.at[0] - a.at[0]
      const dy = b.at[1] - a.at[1]
      const dist = Math.hypot(dx, dy)
      const arc = a.arc ?? 0
      let at: Point
      if (dist > 8 && arc !== 0) {
        const bend = arc * Math.min(240, dist)
        const nx = -dy / dist
        const ny = dx / dist
        const s = 4 * k * (1 - k)
        at = [lerp(a.at[0], b.at[0], k) + nx * bend * s, lerp(a.at[1], b.at[1], k) + ny * bend * s]
      } else {
        at = [lerp(a.at[0], b.at[0], k), lerp(a.at[1], b.at[1], k)]
      }
      return {
        at,
        drawing: false,
        pressed,
        shift,
      }
    }
  }
  return { at: path[path.length - 1].at, drawing: false, pressed, shift }
}

// Splits a narration into its two keyframe clauses.
export const clauses = (mode: Mode) => {
  const { words, clause2 } = NARRATION[mode]
  const text = (ws: typeof words) => ws.map(([w]) => w).join(" ")
  return [text(words.slice(0, clause2)), text(words.slice(clause2))]
}

export const summaryText = (mode: Mode, short = false) => {
  const dir = short ? "~/…/captures" : "~/Library/Application Support/scrawl/captures"
  const target = mode === "mac" ? "Pulse “Deploy Activity”" : "iPhone · dev.pulse.app (212)"
  const [first, second] = clauses(mode)
  const start = recStart(mode)
  return [
    `[scrawl ${clock(T.finish - start)} · ${target} · ${dir}/scrawl-1791924018273-3a1f.scrawl]`,
    `- #1 (${clock(T.pin1 - start)}) “${first}”`,
    `- #2 (${clock(T.pin2 - start)}) “${second}”`,
  ]
}

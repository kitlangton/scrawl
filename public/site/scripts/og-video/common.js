// Shared helpers for the og:video loops. Every concept is a pure function of the
// frame index so a capture is deterministic and frame N === frame 0.
const W = 1200
const H = 630
const FPS = 60

function rng(seed) {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v))
const lerp = (a, b, t) => a + (b - a) * t
const seg = (t, a, b) => clamp((t - a) / (b - a))

function bezier(x1, y1, x2, y2) {
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by
  const sx = (t) => ((ax * t + bx) * t + cx) * t
  const sy = (t) => ((ay * t + by) * t + cy) * t
  const dx = (t) => (3 * ax * t + 2 * bx) * t + cx
  return (x) => {
    if (x <= 0) return 0
    if (x >= 1) return 1
    let t = x
    for (let i = 0; i < 8; i++) {
      const e = sx(t) - x
      if (Math.abs(e) < 1e-6) break
      const d = dx(t)
      if (Math.abs(d) < 1e-6) break
      t -= e / d
    }
    return sy(clamp(t))
  }
}

const ease = {
  enter: bezier(0.2, 0.8, 0.2, 1),
  exit: bezier(0.4, 0, 1, 1),
  draw: bezier(0.45, 0, 0.2, 1),
  inOut: bezier(0.65, 0, 0.35, 1),
}

// Step response of a spring given Kit's (duration, bounce): omega = 2pi/d, zeta = 1 - bounce.
function spring(t, d, bounce = 0) {
  if (t <= 0) return 0
  const w = (2 * Math.PI) / d
  const z = 1 - bounce
  if (z >= 1) return 1 - (1 + w * t) * Math.exp(-w * t)
  const wd = w * Math.sqrt(1 - z * z)
  return 1 - Math.exp(-z * w * t) * (Math.cos(wd * t) + ((z * w) / wd) * Math.sin(wd * t))
}

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath()
  ctx.roundRect(x, y, w, h, r)
}

// Polyline helpers for ink drawn on and wiped off along its own length.
function polyLength(pts) {
  const acc = [0]
  for (let i = 1; i < pts.length; i++) acc.push(acc[i - 1] + Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]))
  return acc
}

function pointAt(pts, acc, s) {
  const total = acc[acc.length - 1]
  s = clamp(s, 0, total)
  let i = 1
  while (i < acc.length - 1 && acc[i] < s) i++
  const k = (s - acc[i - 1]) / (acc[i] - acc[i - 1] || 1)
  return [lerp(pts[i - 1][0], pts[i][0], k), lerp(pts[i - 1][1], pts[i][1], k)]
}

// Draw the part of a polyline between fractions a and b of its length.
function strokePartial(ctx, pts, a, b) {
  if (b <= a) return
  // As a wipe eats the last of a stroke, fade it rather than leave a round-cap dot.
  const fade = a > 0 ? clamp((b - a) / 0.05) : 1
  if (fade <= 0) return
  ctx.save()
  ctx.globalAlpha *= fade
  const acc = polyLength(pts)
  const total = acc[acc.length - 1]
  const s0 = a * total, s1 = b * total
  ctx.beginPath()
  const p0 = pointAt(pts, acc, s0)
  ctx.moveTo(p0[0], p0[1])
  for (let i = 1; i < pts.length; i++) if (acc[i] > s0 && acc[i] < s1) ctx.lineTo(pts[i][0], pts[i][1])
  const p1 = pointAt(pts, acc, s1)
  ctx.lineTo(p1[0], p1[1])
  ctx.stroke()
  ctx.restore()
  return p1
}

// A hand-drawn loop: an ellipse that overshoots its start, with a little wobble.
function handLoop(cx, cy, rx, ry, seed, turns = 1.12, start = -2.2) {
  const r = rng(seed)
  const ph1 = r() * 6, ph2 = r() * 6
  const pts = []
  const n = 90
  for (let i = 0; i <= n; i++) {
    const t = i / n
    const a = start + t * turns * Math.PI * 2
    const wob = 1 + 0.035 * Math.sin(a * 2 + ph1) + 0.02 * Math.sin(a * 3 + ph2)
    const grow = 1 + 0.06 * t
    pts.push([cx + Math.cos(a) * rx * wob * grow, cy + Math.sin(a) * ry * wob * grow])
  }
  return pts
}

function quad(p0, p1, p2, n = 40) {
  const pts = []
  for (let i = 0; i <= n; i++) {
    const t = i / n
    const u = 1 - t
    pts.push([u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]])
  }
  return pts
}

function loadImage(src) {
  return new Promise((res, rej) => {
    const img = new Image()
    img.onload = () => res(img)
    img.onerror = rej
    img.src = src
  })
}

// Matter.js pre-simulation: run `steps` fixed 60 Hz steps and record every body per frame.
function simulate({ steps, setup, onStep, gravity = 1, substeps = 2 }) {
  const { Engine, Composite } = Matter
  const engine = Engine.create({ enableSleeping: false, positionIterations: 10, velocityIterations: 8 })
  engine.gravity.y = gravity
  const world = engine.world
  const ctx = setup(engine, world) || {}
  const frames = []
  for (let f = 0; f < steps; f++) {
    onStep && onStep(f, engine, world, ctx)
    for (let s = 0; s < substeps; s++) Engine.update(engine, 1000 / FPS / substeps)
    const rec = new Map()
    for (const b of Composite.allBodies(world)) if (b.label !== "static") rec.set(b.id, [b.position.x, b.position.y, b.angle])
    frames.push(rec)
  }
  return { frames, ctx }
}

// Apparent helpers.
const INK = "#ff3b30"
const SANS = 'system-ui, -apple-system, "Helvetica Neue", sans-serif'

function inkStyle(ctx, width = 7) {
  ctx.strokeStyle = INK
  ctx.lineWidth = width
  ctx.lineCap = "round"
  ctx.lineJoin = "round"
  ctx.shadowColor = "rgba(0,0,0,0.45)"
  ctx.shadowBlur = 4
  ctx.shadowOffsetY = 1.5
}

// The Apparent brush, tinted to `color` once.
async function loadBrush(color = "#f4f4f5", size = 256) {
  const img = await loadImage("/icon.svg")
  const c = document.createElement("canvas")
  c.width = c.height = size
  const x = c.getContext("2d")
  x.drawImage(img, 0, 0, size, size)
  x.globalCompositeOperation = "source-in"
  x.fillStyle = color
  x.fillRect(0, 0, size, size)
  return c
}

function pin(ctx, x, y, n, s = 1, alpha = 1) {
  ctx.save()
  ctx.globalAlpha *= alpha
  ctx.translate(x, y)
  ctx.scale(s, s)
  ctx.shadowColor = "rgba(0,0,0,0.4)"
  ctx.shadowBlur = 6
  ctx.shadowOffsetY = 2
  ctx.beginPath()
  ctx.arc(0, 0, 17, 0, Math.PI * 2)
  ctx.fillStyle = INK
  ctx.fill()
  ctx.shadowColor = "transparent"
  ctx.lineWidth = 3.5
  ctx.strokeStyle = "#fff"
  ctx.stroke()
  ctx.fillStyle = "#fff"
  ctx.font = `700 20px ${SANS}`
  ctx.textAlign = "center"
  ctx.textBaseline = "middle"
  ctx.fillText(String(n), 0, 1)
  ctx.restore()
}

// Apparent's voice-note bubble: words tick in while the bubble's width leads them.
// t is frames since the bubble started; it exits at t = 86..100.
function bubble(ctx, x, y, text, t, f, { size = 26, exitAt = 86 } = {}) {
  const enter = ease.enter(seg(t, 0, 12))
  const exit = ease.exit(seg(t, exitAt, exitAt + 14))
  const a = enter * (1 - exit)
  if (a <= 0) return
  ctx.save()
  ctx.font = `600 ${size}px ${SANS}`
  const words = text.split(" ")
  const at = (i) => 8 + i * 6
  const h = size * 2.85
  const pad = size * 3.85
  let w = size * 2.6
  words.forEach((word, i) => {
    const ww = ctx.measureText(i ? " " + word : word).width
    w += ww * spring((t - at(i) + 2) / FPS, 0.22)
  })
  w += size * 1.0
  const s = lerp(0.96, 1, enter) * lerp(1, 0.96, exit)
  ctx.globalAlpha = a
  ctx.translate(x, y)
  ctx.scale(s, s)
  ctx.save()
  ctx.shadowColor = "rgba(0,0,0,0.5)"
  ctx.shadowBlur = 30
  ctx.shadowOffsetY = 10
  roundRect(ctx, 0, -h / 2, w, h, size * 0.7)
  ctx.fillStyle = "rgba(30,30,33,0.97)"
  ctx.fill()
  ctx.restore()
  roundRect(ctx, 0, -h / 2, w, h, size * 0.7)
  ctx.strokeStyle = "rgba(255,255,255,0.12)"
  ctx.lineWidth = 1.5
  ctx.stroke()
  const last = at(words.length - 1) + 8
  const speaking = t < 4 ? 0 : clamp(Math.min((t - 4) / 6, (last - t) / 10 + 1))
  for (let i = 0; i < 5; i++) {
    const bh = 8 + 18 * speaking * (0.4 + 0.6 * Math.abs(Math.sin(f * 0.33 + i * 1.9)))
    roundRect(ctx, size * 0.85 + i * 8, -bh / 2, 4, bh, 2)
    ctx.fillStyle = INK
    ctx.fill()
  }
  ctx.textAlign = "left"
  ctx.textBaseline = "middle"
  let wx = size * 2.6
  words.forEach((word, i) => {
    const k = ease.enter(seg(t, at(i), at(i) + 8))
    ctx.fillStyle = `rgba(244,244,245,${k})`
    ctx.fillText(word, wx, 1 + 4 * (1 - k))
    wx += ctx.measureText(word + " ").width
  })
  ctx.restore()
}

// Apparent 3 · Lasso: a busy dashboard. The pen loops one chart, everything else
// dims, and the spoken note says what's wrong. The ink lifts, the light comes
// back, and the pen moves to the next one. Two targets per loop.
;(() => {
  const PER = 165
  const FRAMES = PER * 2
  const TW = 362, TH = 258, GAP = 22
  const OX = (W - 3 * TW - 2 * GAP) / 2
  const OY = (H - 2 * TH - GAP) / 2
  let dimLayer

  const tile = (i) => ({ x: OX + (i % 3) * (TW + GAP), y: OY + Math.floor(i / 3) * (TH + GAP) })

  const TILES = [
    { title: "Revenue", value: "$48.2k", chart: "line", seed: 1 },
    { title: "Signups", value: "1,204", chart: "bars", gap: 4 },
    { title: "Latency", value: "212 ms", chart: "line", seed: 3 },
    { title: "Errors", value: "0.4%", chart: "bars" },
    { title: "Active users", value: "8,941", chart: "line", seed: 5, spike: 15 },
    { title: "Churn", value: "2.1%", chart: "donut" },
  ]

  // Each target: the tile, the loop around its chart, where the note goes, and what it says.
  const TARGETS = [
    { tile: 1, note: "Tuesday is missing.", side: 1 },
    { tile: 4, note: "What's this spike?", side: 1 },
  ]

  function chartBox(i) {
    const { x, y } = tile(i)
    return { x: x + 24, y: y + 118, w: TW - 48, h: 110 }
  }

  function drawTile(ctx, i) {
    const t = TILES[i]
    const { x, y } = tile(i)
    roundRect(ctx, x, y, TW, TH, 18)
    ctx.fillStyle = "#18181b"
    ctx.fill()
    ctx.strokeStyle = "rgba(255,255,255,0.07)"
    ctx.lineWidth = 1.5
    ctx.stroke()
    ctx.textAlign = "left"
    ctx.textBaseline = "alphabetic"
    ctx.fillStyle = "#8e8e93"
    ctx.font = `500 19px ${SANS}`
    ctx.fillText(t.title, x + 24, y + 40)
    ctx.fillStyle = "#f4f4f5"
    ctx.font = `600 38px ${SANS}`
    ctx.fillText(t.value, x + 24, y + 88)
    const b = chartBox(i)
    if (t.chart === "bars") {
      const n = 7, bw = (b.w - (n - 1) * 12) / n
      for (let k = 0; k < n; k++) {
        const hh = k === t.gap ? 4 : b.h * (0.35 + 0.6 * Math.abs(Math.sin(k * 1.3 + i)))
        roundRect(ctx, b.x + k * (bw + 12), b.y + b.h - hh, bw, hh, 5)
        ctx.fillStyle = k === t.gap ? "#2c2c2f" : "#5e5ce6"
        ctx.fill()
      }
    } else if (t.chart === "line") {
      ctx.beginPath()
      for (let k = 0; k <= 24; k++) {
        const px = b.x + (k / 24) * b.w
        const amp = t.spike != null ? 0.45 : 1
        let v = 0.5 + amp * (0.22 * Math.sin(k * 0.55 + t.seed) + 0.12 * Math.sin(k * 1.7 + t.seed * 2)) + k * 0.008
        if (t.spike != null) v += k === t.spike ? 0.5 : 0
        const py = b.y + b.h - clamp(v, 0.05, 1.05) * b.h
        k ? ctx.lineTo(px, py) : ctx.moveTo(px, py)
      }
      ctx.strokeStyle = "#64d2ff"
      ctx.lineWidth = 3.5
      ctx.lineJoin = "round"
      ctx.stroke()
    } else {
      const cx = b.x + b.w / 2, cy = b.y + b.h / 2 - 4, rr = 50
      ctx.lineWidth = 18
      ctx.beginPath()
      ctx.arc(cx, cy, rr, 0, Math.PI * 2)
      ctx.strokeStyle = "#2c2c2f"
      ctx.stroke()
      ctx.beginPath()
      ctx.arc(cx, cy, rr, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * 0.68)
      ctx.strokeStyle = "#ff9f0a"
      ctx.stroke()
    }
  }

  register({
    frames: FRAMES,
    async init() {
      dimLayer = document.createElement("canvas")
      dimLayer.width = W
      dimLayer.height = H
      TARGETS.forEach((tg, i) => {
        const b = chartBox(tg.tile)
        tg.cx = b.x + b.w / 2
        tg.cy = b.y + b.h / 2 - 8
        tg.loop = handLoop(tg.cx, tg.cy, b.w / 2 + 34, b.h / 2 + 30, 41 + i * 7, 1.1, tg.side > 0 ? -2.5 : -0.7)
      })
    },
    draw(ctx, f) {
      ctx.fillStyle = "#0a0a0b"
      ctx.fillRect(0, 0, W, H)
      for (let i = 0; i < 6; i++) drawTile(ctx, i)

      const ti = Math.floor(f / PER)
      const t = f - ti * PER
      const tg = TARGETS[ti]
      const draw = ease.draw(seg(t, 6, 34))
      const wipe = ease.inOut(seg(t, 122, 142))
      const dim = ease.enter(seg(t, 26, 46)) * (1 - ease.inOut(seg(t, 124, 146)))

      // Dim everything outside the loop, with a soft edge.
      if (dim > 0) {
        const d = dimLayer.getContext("2d")
        d.clearRect(0, 0, W, H)
        d.globalCompositeOperation = "source-over"
        d.fillStyle = `rgba(4,4,6,${0.62 * dim})`
        d.fillRect(0, 0, W, H)
        d.globalCompositeOperation = "destination-out"
        d.filter = "blur(16px)"
        d.beginPath()
        tg.loop.forEach((p, i) => (i ? d.lineTo(p[0], p[1]) : d.moveTo(p[0], p[1])))
        d.closePath()
        d.fillStyle = "#000"
        d.fill()
        d.filter = "none"
        ctx.drawImage(dimLayer, 0, 0)
      }

      ctx.save()
      inkStyle(ctx, 7)
      strokePartial(ctx, tg.loop, wipe, draw)
      ctx.restore()

      const b = chartBox(tg.tile)
      const nx = tg.side > 0 ? b.x + b.w + 44 : b.x - 44
      const ny = tg.cy + (tg.tile === 1 ? 70 : -98)
      ctx.save()
      ctx.font = `600 26px ${SANS}`
      const est = ctx.measureText(tg.note).width + 26 * 3.6
      ctx.restore()
      bubble(ctx, tg.side > 0 ? nx - 40 : nx - est + 40, ny, tg.note, t - 36, f)
    },
  })
})()

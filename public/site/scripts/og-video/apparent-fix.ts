// Apparent 2 · Fix: a small app window slides in, a red mark lands on the problem,
// the spoken note ticks in, and the app fixes itself. The ink lifts, the window
// slides away and the next one arrives. Two windows; the seam is mid-belt.
;(() => {
  const PER = 180
  const FRAMES = PER * 2
  const SPACING = 1200
  const CARD = { w: 600, h: 400, cx: 408, cy: 315 }
  const Z = 1.2
  let cards

  const COPY = ["These two don't line up.", "This should be green."]

  function windowBase(ctx, title, sub) {
    const { w, h } = CARD
    ctx.save()
    ctx.shadowColor = "rgba(0,0,0,0.55)"
    ctx.shadowBlur = 50
    ctx.shadowOffsetY = 20
    roundRect(ctx, -w / 2, -h / 2, w, h, 20)
    ctx.fillStyle = "#1a1a1d"
    ctx.fill()
    ctx.restore()
    roundRect(ctx, -w / 2, -h / 2, w, h, 20)
    ctx.strokeStyle = "rgba(255,255,255,0.09)"
    ctx.lineWidth = 1.5
    ctx.stroke()
    for (let i = 0; i < 3; i++) {
      ctx.beginPath()
      ctx.arc(-w / 2 + 26 + i * 20, -h / 2 + 24, 6, 0, Math.PI * 2)
      ctx.fillStyle = "#3a3a3d"
      ctx.fill()
    }
    ctx.textAlign = "left"
    ctx.textBaseline = "alphabetic"
    ctx.fillStyle = "#f4f4f5"
    ctx.font = `600 30px ${SANS}`
    ctx.fillText(title, -w / 2 + 28, -h / 2 + 84)
    ctx.fillStyle = "#8e8e93"
    ctx.font = `400 18px ${SANS}`
    ctx.fillText(sub, -w / 2 + 28, -h / 2 + 110)
  }

  function button(ctx, x, y, w, label, primary) {
    roundRect(ctx, x, y, w, 44, 11)
    ctx.fillStyle = primary ? "#0a84ff" : "#2c2c2f"
    ctx.fill()
    ctx.fillStyle = "#fff"
    ctx.font = `600 19px ${SANS}`
    ctx.textAlign = "center"
    ctx.textBaseline = "middle"
    ctx.fillText(label, x + w / 2, y + 23)
  }

  // Card A: two buttons, the right one 16 px low until fixed.
  function cardA(ctx, fix) {
    windowBase(ctx, "Deploys", "main · 3 services")
    const rows = ["api-gateway", "billing-worker", "web-console"]
    rows.forEach((name, i) => {
      const y = -20 + i * 50
      ctx.fillStyle = "#d1d1d6"
      ctx.font = `500 19px ui-monospace, "SF Mono", Menlo, monospace`
      ctx.textAlign = "left"
      ctx.textBaseline = "middle"
      ctx.fillText(name, -CARD.w / 2 + 28, y)
      ctx.beginPath()
      ctx.arc(CARD.w / 2 - 128, y, 5.5, 0, Math.PI * 2)
      ctx.fillStyle = "#30d158"
      ctx.fill()
      ctx.fillStyle = "#aeaeb2"
      ctx.font = `400 18px ${SANS}`
      ctx.fillText("Healthy", CARD.w / 2 - 114, y + 1)
      ctx.fillStyle = "rgba(255,255,255,0.06)"
      ctx.fillRect(-CARD.w / 2 + 28, y + 23, CARD.w - 56, 1)
    })
    const by = CARD.h / 2 - 76
    button(ctx, CARD.w / 2 - 300, by, 124, "Retry", false)
    button(ctx, CARD.w / 2 - 164, by + 22 * (1 - fix), 136, "Promote", true)
  }

  // Card B: the success rate is red until fixed.
  function cardB(ctx, fix) {
    windowBase(ctx, "Overview", "Last 24 hours")
    const tiles = [["Deploys", "128"], ["Success", "97.6%"], ["Median", "4m 12s"]]
    const tw = 160, gap = 14
    tiles.forEach(([k, v], i) => {
      const x = -CARD.w / 2 + 28 + i * (tw + gap)
      const y = -28
      roundRect(ctx, x, y, tw, 104, 14)
      ctx.fillStyle = "#232326"
      ctx.fill()
      ctx.fillStyle = "#8e8e93"
      ctx.font = `400 17px ${SANS}`
      ctx.textAlign = "left"
      ctx.textBaseline = "alphabetic"
      ctx.fillText(k, x + 16, y + 32)
      const red = [255, 69, 58], green = [48, 209, 88]
      const c = i === 1 ? red.map((r, j) => Math.round(lerp(r, green[j], fix))) : [244, 244, 245]
      ctx.fillStyle = `rgb(${c})`
      ctx.font = `600 32px ${SANS}`
      ctx.fillText(v, x + 16, y + 78)
    })
    // A sparkline under the tiles.
    ctx.strokeStyle = "#0a84ff"
    ctx.lineWidth = 3
    ctx.lineJoin = "round"
    ctx.beginPath()
    for (let i = 0; i <= 24; i++) {
      const x = -CARD.w / 2 + 28 + (i / 24) * (CARD.w - 56)
      const y = CARD.h / 2 - 50 - 16 * Math.sin(i * 0.7) * Math.cos(i * 0.23) - i * 0.6
      i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)
    }
    ctx.stroke()
  }

  register({
    frames: FRAMES,
    async init() {
      cards = [cardA, cardB]
    },
    draw(ctx, f) {
      ctx.fillStyle = "#0a0a0b"
      ctx.fillRect(0, 0, W, H)
      const g = ctx.createRadialGradient(600, 300, 0, 600, 300, 800)
      g.addColorStop(0, "#1a1418")
      g.addColorStop(1, "#0a0a0b")
      ctx.fillStyle = g
      ctx.fillRect(0, 0, W, H)

      const cycle = Math.floor(f / PER)
      const local = f - cycle * PER
      const shift = ease.inOut(seg(local, 146, 180))

      for (let k = 0; k < 2; k++) {
        const idx = (cycle + k) % 2
        const ox = (k - shift) * SPACING
        if (ox < -W || ox > W) continue
        const active = k === 0
        const fix = active ? spring((local - 100) / FPS, 0.3) : 0
        ctx.save()
        ctx.translate(CARD.cx + ox, CARD.cy)
        ctx.scale(Z, Z)
        cards[idx](ctx, fix)
        if (active) annotate(ctx, idx, local, f)
        ctx.restore()
      }
    },
  })

  // The note bubble is drawn at screen scale beside the zoomed window.
  function note(ctx, x, y, text, t, f) {
    ctx.save()
    ctx.scale(1 / Z, 1 / Z)
    bubble(ctx, x * Z, y * Z, text, t, f)
    ctx.restore()
  }

  function annotate(ctx, idx, t, f) {
    const draw = ease.draw(seg(t, 8, 34))
    const wipe = ease.inOut(seg(t, 118, 138))
    if (idx === 0) {
      const by = CARD.h / 2 - 76 + 30
      const loop = handLoop(CARD.w / 2 - 164, by, 186, 54, 5, 1.1, -2.6)
      ctx.save()
      inkStyle(ctx, 7)
      strokePartial(ctx, loop, wipe, draw)
      ctx.restore()
      const pp = ease.enter(seg(t, 32, 44)) * (1 - ease.exit(seg(t, 120, 132)))
      pin(ctx, CARD.w / 2 + 6, by - 44, 1, lerp(0.85, 1, pp), pp)
      note(ctx, CARD.w / 2 - 6, by + 4, COPY[0], t - 30, f)
    } else {
      const tail = [CARD.w / 2 - 34, 40]
      const head = [-CARD.w / 2 + 28 + 174 + 92, -28 + 104 + 12]
      const shaft = quad(tail, [tail[0] - 120, head[1] + 60], head, 36)
      const tip = shaft.at(-1)
      const dir = Math.atan2(tip[1] - shaft.at(-4)[1], tip[0] - shaft.at(-4)[0])
      const arm = (a) => [tip[0] - Math.cos(dir + a) * 26, tip[1] - Math.sin(dir + a) * 26]
      const headPts = [arm(0.55), tip, arm(-0.55)]
      ctx.save()
      inkStyle(ctx, 7)
      const shaftDraw = clamp(draw / 0.78)
      const headDraw = clamp((draw - 0.78) / 0.22)
      strokePartial(ctx, shaft, wipe, shaftDraw)
      if (headDraw > 0) strokePartial(ctx, headPts, wipe, headDraw)
      ctx.restore()
      const pp = ease.enter(seg(t, 4, 16)) * (1 - ease.exit(seg(t, 120, 132)))
      pin(ctx, tail[0] + 10, tail[1] - 30, 1, lerp(0.85, 1, pp), pp)
      note(ctx, CARD.w / 2 - 6, tail[1] + 4, COPY[1], t - 30, f)
    }
  }
})()

// Apparent 1 · Pile: marks get drawn in the air (circles, arrows, checks, pins),
// then drop and pile up under the brush. The floor lets go, the pile falls away,
// and the empty frame is the first frame again.
;(() => {
  const FRAMES = 280
  const FLOOR_DROP = 214
  const LOGO = { x: 600, y: 238 }
  const DRAW = 11
  let brush, sim, marks

  function geometry(kind, r) {
    switch (kind) {
      case "loop": {
        const rx = 38 + r() * 30, ry = 24 + r() * 16
        return { strokes: [handLoop(0, 0, rx, ry, (r() * 1e6) | 0, 1.1, -2.4)], body: ["rect", rx * 2 + 10, ry * 2 + 10, ry] }
      }
      case "arrow": {
        const L = 80 + r() * 50, bend = (r() - 0.5) * 30
        const shaft = quad([-L / 2, 8], [0, 8 + bend], [L / 2, -4], 24)
        const tip = shaft[shaft.length - 1]
        const head = [[tip[0] - 20, tip[1] - 12], tip, [tip[0] - 14, tip[1] + 16]]
        return { strokes: [shaft, head], body: ["rect", L + 12, 40, 10] }
      }
      case "under": {
        const L = 90 + r() * 50
        const pts = []
        for (let i = 0; i <= 30; i++) pts.push([-L / 2 + (L * i) / 30, Math.sin(i * 0.75) * 6])
        return { strokes: [pts], body: ["rect", L + 10, 24, 10] }
      }
      case "check":
        return { strokes: [[[-22, 0], [-7, 16], [24, -20]]], body: ["rect", 58, 46, 10] }
      case "cross":
        return { strokes: [[[-17, -17], [17, 17]], [[17, -17], [-17, 17]]], body: ["rect", 44, 44, 12] }
      case "pin":
        return { strokes: [], pin: 1 + ((r() * 4) | 0), body: ["circle", 19] }
    }
  }

  const KINDS = ["loop", "arrow", "loop", "under", "check", "pin", "arrow", "cross", "loop", "pin", "check", "under"]

  register({
    frames: FRAMES,
    async init(ctx) {
      brush = await loadBrush()
      const r = rng(23)
      marks = []
      let f = 2
      for (let i = 0; i < 46; i++) {
        const kind = KINDS[i % KINDS.length]
        const g = geometry(kind, r)
        const side = i % 2 ? 1 : -1
        marks.push({
          ...g,
          kind,
          at: f,
          x: W / 2 + side * (60 + r() * 460),
          y: 70 + r() * 90,
          a: (r() - 0.5) * 0.7,
          vx: (r() - 0.5) * 3,
          av: (r() - 0.5) * 0.1,
        })
        f += 3
      }
      sim = simulate({
        steps: FRAMES,
        gravity: 2.3,
        setup(engine, world) {
          const { Bodies, Composite } = Matter
          const opts = { isStatic: true, label: "static", friction: 0.8 }
          const floor = Bodies.rectangle(W / 2, H - 10 + 50, W * 2, 100, opts)
          Composite.add(world, [floor, Bodies.rectangle(-50, 0, 100, H * 4, opts), Bodies.rectangle(W + 50, 0, 100, H * 4, opts)])
          return { floor }
        },
        onStep(f, engine, world, c) {
          const { Bodies, Body, Composite } = Matter
          if (f === FLOOR_DROP) Composite.remove(world, c.floor)
          for (const m of marks) {
            if (m.at + DRAW !== f) continue
            const o = { friction: 0.6, frictionStatic: 0.9, restitution: 0.15, density: 0.002, angle: m.a }
            const b = m.body[0] === "circle" ? Bodies.circle(m.x, m.y, m.body[1], o) : Bodies.rectangle(m.x, m.y, m.body[1], m.body[2], { ...o, chamfer: { radius: m.body[3] } })
            Body.setVelocity(b, { x: m.vx, y: 0 })
            Body.setAngularVelocity(b, m.av)
            m.id = b.id
            Composite.add(world, b)
          }
        },
      })
    },
    draw(ctx, f) {
      ctx.fillStyle = "#0a0a0b"
      ctx.fillRect(0, 0, W, H)
      const g = ctx.createRadialGradient(LOGO.x, LOGO.y, 0, LOGO.x, LOGO.y, 700)
      g.addColorStop(0, "rgba(255,255,255,0.045)")
      g.addColorStop(1, "rgba(255,255,255,0)")
      ctx.fillStyle = g
      ctx.fillRect(0, 0, W, H)

      const rec = sim.frames[f]
      for (const m of marks) {
        if (f < m.at) continue
        const p = rec.get(m.id) ?? [m.x, m.y, m.a]
        if (p[1] > H + 120) continue
        const progress = ease.draw(seg(f, m.at, m.at + DRAW))
        ctx.save()
        ctx.translate(p[0], p[1])
        ctx.rotate(p[2])
        if (m.pin) {
          pin(ctx, 0, 0, m.pin, lerp(0.8, 1, ease.enter(progress)), progress)
        } else {
          inkStyle(ctx, 7)
          const lens = m.strokes.map((s) => polyLength(s).at(-1))
          const total = lens.reduce((a, b) => a + b, 0)
          let done = progress * total
          m.strokes.forEach((s, i) => {
            const k = clamp(done / lens[i])
            if (k > 0) strokePartial(ctx, s, 0, k)
            done -= lens[i]
          })
        }
        ctx.restore()
      }

      // Brush + name, drawn over the marks.
      ctx.save()
      ctx.shadowColor = "rgba(0,0,0,0.7)"
      ctx.shadowBlur = 24
      const bs = 74
      ctx.drawImage(brush, LOGO.x - 132, LOGO.y - bs / 2, bs, bs)
      ctx.fillStyle = "#f4f4f5"
      ctx.font = `600 54px ${SANS}`
      ctx.textAlign = "left"
      ctx.textBaseline = "middle"
      ctx.fillText("Apparent", LOGO.x - 44, LOGO.y + 2)
      ctx.restore()
    },
  })
})()

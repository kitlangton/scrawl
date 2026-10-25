import AppKit
import Combine
import SwiftUI

// The how-it-works screen's demo: a silent loop on a pretend checkout page with a button that
// runs out of its window. ⌥⇧S opens the pill, words appear as if spoken, ⇧ draws a red mark
// around the bug, a click points at the edge it should match, ⌥⇧S finishes, the frame lifts off
// with its `1` chip, ⌥⇧V drops it into an agent chat and the agent fixes the button.
//
// Everything on the stage is a function of one clock (`DemoScript`), so a frame at any time can
// be rendered (`--render`) and the loop always looks the same. Holding ⇧ over the stage pauses
// the loop and draws a mark there (in this window only; nothing is recorded). Reduced motion
// shows four still frames instead. Springs are bounce 0, by duration (ω = 2π / d).

// MARK: Script

enum DemoScript {
  static let loop = 11.0
  /// ⌥⇧S goes down, then the pill opens.
  static let startKeys = 0.5
  static let pillOpen = 0.62
  static let recordStart = 0.7
  static let approach = (1.0, 1.9)
  static let mark = (2.6, 3.9)
  static let toTotal = (4.2, 4.8)
  static let click = 4.9
  static let finishKeys = 5.8
  /// The pill turns to Copied and the frame lifts off the page.
  static let finish = 6.0
  static let toComposer = (6.2, 6.85)
  static let composerClick = 6.9
  static let pasteKeys = 7.0
  static let fly = 7.05
  static let pillClose = 7.55
  static let send = 7.95
  static let reply = 8.7
  static let fix = 9.0
  /// The story ends here (the button reads Continue after one play-through).
  static let end = 10.4
  /// Back to the start: the chat clears, the button slides back out, the pointer returns.
  static let reset = 10.4
  static let resetCaption = 10.6

  /// Spoken words (shown, never heard) with the time each one is said.
  static let sentences: [(shown: (Double, Double), words: [(String, Double)])] = [
    ((1.2, 2.5), [("This", 1.25), ("button", 1.42), ("is", 1.62), ("cut", 1.75), ("off", 1.92)]),
    ((2.6, 4.15), [("and", 2.7), ("too", 2.9), ("far", 3.08), ("right.", 3.28)]),
    ((4.3, 5.9), [("Line", 4.35), ("it", 4.5), ("up", 4.62), ("with", 4.78), ("this.", 4.95)]),
  ]
  static let transcript = "This button is cut off and too far right. Line it up with this."
  static let replyText = "Moved the button back in line."
}

func clamp01(_ x: Double) -> Double { min(1, max(0, x)) }
func ramp(_ t: Double, _ a: Double, _ b: Double) -> Double { clamp01((t - a) / (b - a)) }
func smooth(_ p: Double) -> Double { p * p * (3 - 2 * p) }
func easeInOut(_ p: Double) -> Double { p < 0.5 ? 4 * p * p * p : 1 - pow(-2 * p + 2, 3) / 2 }

/// A critically damped spring (bounce 0) released at `start`, 0 → 1, settled after `duration`.
func springStep(_ t: Double, _ start: Double, _ duration: Double) -> Double {
  let tau = t - start
  guard tau > 0 else { return 0 }
  let w = 2 * Double.pi / duration
  return min(1, 1 - (1 + w * tau) * exp(-w * tau))
}

func mix(_ a: CGFloat, _ b: CGFloat, _ p: Double) -> CGFloat { a + (b - a) * CGFloat(p) }
func mix(_ a: CGPoint, _ b: CGPoint, _ p: Double) -> CGPoint { CGPoint(x: mix(a.x, b.x, p), y: mix(a.y, b.y, p)) }
func mix(_ a: CGRect, _ b: CGRect, _ p: Double) -> CGRect {
  CGRect(x: mix(a.minX, b.minX, p), y: mix(a.minY, b.minY, p), width: mix(a.width, b.width, p), height: mix(a.height, b.height, p))
}

// MARK: Geometry (stage points, top-left origin)

enum DemoGeo {
  static let size = CGSize(width: 540, height: 244)
  static let app = CGRect(x: 14, y: 50, width: 300, height: 180)
  static let agent = CGRect(x: 326, y: 50, width: 200, height: 180)
  /// The Pay button in the window's own coordinates: where it belongs, and where the bug put it
  /// (its right end runs past the window, cutting "Pay now" short).
  static let buttonFixed = CGRect(x: 200, y: 122, width: 84, height: 26)
  static let buttonBroken = CGRect(x: 250, y: 126, width: 84, height: 26)
  static let markCenter = CGPoint(x: 292, y: 189)
  static let markRadius = CGSize(width: 40, height: 23)
  static let clickPoint = CGPoint(x: 297, y: 156)
  static let rest = CGPoint(x: 170, y: 118)
  static let composer = CGRect(x: 334, y: 194, width: 184, height: 28)
  static let composerPoint = CGPoint(x: 452, y: 210)
  static let composerThumb = CGRect(x: 339, y: 198, width: 33.3, height: 20)
  static let bubble = CGRect(x: 348, y: 82, width: 170, height: 84)
  static let bubbleThumb = CGRect(x: 354, y: 88, width: 72, height: 43.2)
  static let pillTop: CGFloat = 10
  static let pillHeight: CGFloat = 30
  static let pillClosed: CGFloat = 30
  static let pillRecording: CGFloat = 116
  static let pillCopied: CGFloat = 128

  /// A hand-drawn loop around the cut-off end of the button, a little more than one turn.
  static func markPoint(_ p: Double) -> CGPoint {
    let theta = (200.0 / 180 * Double.pi) + p * 2.25 * Double.pi
    let wobble = 1 + 0.07 * sin(3 * theta + 0.6) + 0.05 * p
    return CGPoint(
      x: markCenter.x + markRadius.width * CGFloat(cos(theta) * wobble),
      y: markCenter.y + markRadius.height * CGFloat(sin(theta) * wobble)
    )
  }

  static func markPath(upTo p: Double) -> Path {
    var path = Path()
    let steps = max(2, Int(80 * p))
    for i in 0...steps {
      let point = markPoint(p * Double(i) / Double(steps))
      if i == 0 { path.move(to: point) } else { path.addLine(to: point) }
    }
    return path
  }
}

// MARK: Palette for the pretend page (light, so the red ink reads as it does on a real site)

enum DemoInk {
  static let page = Color(red: 0.972, green: 0.972, blue: 0.965)
  static let titleBar = Color(red: 0.93, green: 0.93, blue: 0.925)
  static let text = Color(red: 0.11, green: 0.11, blue: 0.12)
  static let secondary = Color(red: 0.36, green: 0.36, blue: 0.38)
  static let pill = Color(red: 20 / 255, green: 20 / 255, blue: 22 / 255).opacity(0.96)
  static let accent = Color(red: 0x3B / 255, green: 0x5C / 255, blue: 0xF6 / 255)
  static let agent = Color(red: 0x23 / 255, green: 0x23 / 255, blue: 0x25 / 255)
  static let sheet = Color(red: 0x0E / 255, green: 0x0E / 255, blue: 0x0E / 255)
}

// MARK: Drawing

struct DemoScene {
  let t: Double

  /// 0 while the button is broken, 1 once the agent has fixed it.
  var fixed: Double { springStep(t, DemoScript.fix, 0.45) * (1 - springStep(t, DemoScript.reset, 0.45)) }

  /// 0 … 1 while the loop clears back to its first frame.
  var reset: Double { smooth(ramp(t, DemoScript.reset, DemoScript.reset + 0.35)) }

  /// The arrow the demo moves (the person's pointer), and whether it's pressed.
  var cursor: (point: CGPoint, pressed: Bool) {
    let s = DemoScript.self
    let markStart = DemoGeo.markPoint(0)
    let markEnd = DemoGeo.markPoint(1)
    let pressed = (s.click..<s.click + 0.14).contains(t) || (s.composerClick..<s.composerClick + 0.14).contains(t)
    let point: CGPoint
    if t < s.approach.0 {
      point = DemoGeo.rest
    } else if t < s.approach.1 {
      point = mix(DemoGeo.rest, markStart, easeInOut(ramp(t, s.approach.0, s.approach.1)))
    } else if t < s.mark.0 {
      point = markStart
    } else if t < s.mark.1 {
      point = DemoGeo.markPoint(smooth(ramp(t, s.mark.0, s.mark.1)))
    } else if t < s.toTotal.0 {
      point = markEnd
    } else if t < s.toTotal.1 {
      point = mix(markEnd, DemoGeo.clickPoint, easeInOut(ramp(t, s.toTotal.0, s.toTotal.1)))
    } else if t < s.toComposer.0 {
      point = DemoGeo.clickPoint
    } else if t < s.reset {
      point = mix(DemoGeo.clickPoint, DemoGeo.composerPoint, easeInOut(ramp(t, s.toComposer.0, s.toComposer.1)))
    } else {
      point = mix(DemoGeo.composerPoint, DemoGeo.rest, easeInOut(ramp(t, s.reset, s.loop - 0.05)))
    }
    return (point, pressed)
  }

  var markProgress: Double { smooth(ramp(t, DemoScript.mark.0, DemoScript.mark.1)) }

  /// The lifted frame's rect on the stage, or nil before ⌥⇧S finishes.
  var frameRect: CGRect? {
    let s = DemoScript.self
    guard t >= s.finish else { return nil }
    let lift = springStep(t, s.finish, 0.38)
    let scale = 1 - 0.1 * lift
    let lifted = CGRect(
      x: DemoGeo.app.midX - DemoGeo.app.width * scale / 2,
      y: DemoGeo.app.midY - DemoGeo.app.height * scale / 2,
      width: DemoGeo.app.width * scale, height: DemoGeo.app.height * scale
    )
    guard t >= s.fly else { return lifted }
    // Along an arc into the composer, shrinking as it goes.
    let q = springStep(t, s.fly, 0.6)
    let a = CGPoint(x: lifted.midX, y: lifted.midY)
    let b = CGPoint(x: DemoGeo.composerThumb.midX, y: DemoGeo.composerThumb.midY)
    let control = CGPoint(x: (a.x + b.x) / 2, y: min(a.y, b.y) - 70)
    let u = CGFloat(q)
    let center = CGPoint(
      x: (1 - u) * (1 - u) * a.x + 2 * (1 - u) * u * control.x + u * u * b.x,
      y: (1 - u) * (1 - u) * a.y + 2 * (1 - u) * u * control.y + u * u * b.y
    )
    let width = mix(lifted.width, DemoGeo.composerThumb.width, q)
    let height = mix(lifted.height, DemoGeo.composerThumb.height, q)
    let inComposer = CGRect(x: center.x - width / 2, y: center.y - height / 2, width: width, height: height)
    guard t >= s.send else { return inComposer }
    return mix(inComposer, DemoGeo.bubbleThumb, springStep(t, s.send, 0.42))
  }

  func draw(_ ctx: inout GraphicsContext) {
    let s = DemoScript.self
    let stage = CGRect(origin: .zero, size: DemoGeo.size)
    let stageShape = Path(roundedRect: stage, cornerRadius: 12, style: .continuous)

    ctx.fill(stageShape, with: .linearGradient(
      Gradient(colors: [Color(red: 0.13, green: 0.13, blue: 0.135), Color(red: 0.10, green: 0.10, blue: 0.105)]),
      startPoint: .zero, endPoint: CGPoint(x: 0, y: stage.height)
    ))
    ctx.clip(to: stageShape)

    drawAgent(&ctx)

    // The page, with a drop shadow under its window only.
    ctx.drawLayer { layer in
      layer.addFilter(.shadow(color: .black.opacity(0.45), radius: 9, y: 3))
      layer.fill(Path(roundedRect: DemoGeo.app, cornerRadius: 8, style: .continuous), with: .color(DemoInk.page))
    }
    ctx.drawLayer { layer in
      layer.translateBy(x: DemoGeo.app.minX, y: DemoGeo.app.minY)
      drawPage(&layer, fixed: fixed)
    }

    // Live ink and the click ripple, until the frame lifts off and takes them with it.
    if t < s.finish {
      if markProgress > 0 {
        ctx.stroke(DemoGeo.markPath(upTo: markProgress), with: .color(Palette.ink), style: StrokeStyle(lineWidth: 3.2, lineCap: .round, lineJoin: .round))
      }
      let r = ramp(t, s.click, s.click + 0.6)
      if r > 0 && r < 1 {
        let radius = 4 + 13 * CGFloat(1 - pow(1 - r, 3))
        let p = DemoGeo.clickPoint
        ctx.stroke(Path(ellipseIn: CGRect(x: p.x - radius, y: p.y - radius, width: radius * 2, height: radius * 2)),
                   with: .color(Palette.ink.opacity(1 - r)), lineWidth: 2)
      }
    }

    drawSubtitle(&ctx)

    if let rect = frameRect, reset < 1 {
      ctx.drawLayer { layer in
        layer.opacity = 1 - reset
        if reset > 0 { layer.addFilter(.blur(radius: 3 * reset)) }
        drawFrame(&layer, in: rect)
      }
    }

    drawPill(&ctx)

    // The start cue: a brief red glow around the edge of the screen being recorded.
    let glow = 0.7 * ramp(t, s.pillOpen, s.pillOpen + 0.1) * (1 - smooth(ramp(t, s.pillOpen + 0.15, s.pillOpen + 0.8)))
    if glow > 0 {
      ctx.drawLayer { layer in
        layer.addFilter(.blur(radius: 4))
        layer.stroke(Path(roundedRect: stage.insetBy(dx: 1, dy: 1), cornerRadius: 11, style: .continuous),
                     with: .color(Palette.ink.opacity(0.85 * glow)), lineWidth: 4)
      }
    }
  }

  /// The pretend checkout page, in its own 300 × 180 coordinates.
  func drawPage(_ ctx: inout GraphicsContext, fixed: Double) {
    let w = DemoGeo.app.width, h = DemoGeo.app.height
    let window = Path(roundedRect: CGRect(x: 0, y: 0, width: w, height: h), cornerRadius: 8, style: .continuous)
    ctx.fill(window, with: .color(DemoInk.page))
    ctx.drawLayer { page in
      page.clip(to: window)
      page.fill(Path(CGRect(x: 0, y: 0, width: w, height: 22)), with: .color(DemoInk.titleBar))
      page.fill(Path(CGRect(x: 0, y: 22, width: w, height: 0.5)), with: .color(.black.opacity(0.1)))
      for (i, color) in [Color(red: 1, green: 0.37, blue: 0.34), Color(red: 1, green: 0.74, blue: 0.18), Color(red: 0.16, green: 0.78, blue: 0.25)].enumerated() {
        let x = 11 + CGFloat(i) * 10
        page.fill(Path(ellipseIn: CGRect(x: x - 3.2, y: 11 - 3.2, width: 6.4, height: 6.4)), with: .color(color.opacity(0.9)))
      }
      page.draw(Text("Checkout").font(.system(size: 9, weight: .medium)).foregroundColor(DemoInk.secondary), at: CGPoint(x: w / 2, y: 11))

      page.draw(Text("Order summary").font(.system(size: 11, weight: .semibold)).foregroundColor(DemoInk.text), at: CGPoint(x: 16, y: 40), anchor: .leading)
      let rows: [(String, String, CGFloat)] = [("Notebook", "$24.00", 62), ("Shipping", "$4.00", 80)]
      for (label, price, y) in rows {
        page.draw(Text(label).font(.system(size: 9.5)).foregroundColor(DemoInk.secondary), at: CGPoint(x: 16, y: y), anchor: .leading)
        page.draw(Text(price).font(.system(size: 9.5).monospacedDigit()).foregroundColor(DemoInk.secondary), at: CGPoint(x: 284, y: y), anchor: .trailing)
      }
      page.fill(Path(CGRect(x: 16, y: 92, width: 268, height: 0.5)), with: .color(.black.opacity(0.12)))
      page.draw(Text("Total").font(.system(size: 10, weight: .semibold)).foregroundColor(DemoInk.text), at: CGPoint(x: 16, y: 106), anchor: .leading)
      page.draw(Text("$28.00").font(.system(size: 10, weight: .semibold).monospacedDigit()).foregroundColor(DemoInk.text), at: CGPoint(x: 284, y: 106), anchor: .trailing)

      let button = mix(DemoGeo.buttonBroken, DemoGeo.buttonFixed, fixed)
      page.fill(Path(roundedRect: button, cornerRadius: 7, style: .continuous), with: .color(DemoInk.text))
      page.draw(Text("Pay now").font(.system(size: 9.5, weight: .semibold)).foregroundColor(.white), at: CGPoint(x: button.midX, y: button.midY))
    }
    ctx.stroke(window, with: .color(.black.opacity(0.25)), lineWidth: 0.5)
  }

  func drawAgent(_ ctx: inout GraphicsContext) {
    let s = DemoScript.self
    let g = DemoGeo.agent
    let panel = Path(roundedRect: g, cornerRadius: 8, style: .continuous)
    ctx.drawLayer { layer in
      layer.addFilter(.shadow(color: .black.opacity(0.4), radius: 8, y: 3))
      layer.fill(panel, with: .color(DemoInk.agent))
    }
    ctx.stroke(panel, with: .color(.white.opacity(0.09)), lineWidth: 0.5)
    ctx.fill(Path(ellipseIn: CGRect(x: g.minX + 11, y: g.minY + 9, width: 5, height: 5)), with: .color(Palette.positive.opacity(0.9)))
    ctx.draw(Text("Agent").font(.system(size: 9.5, weight: .semibold)).foregroundColor(Palette.soft), at: CGPoint(x: g.minX + 21, y: g.minY + 11.5), anchor: .leading)
    ctx.fill(Path(CGRect(x: g.minX, y: g.minY + 23, width: g.width, height: 0.5)), with: .color(.white.opacity(0.07)))

    // The composer: a placeholder, then the pasted frame and words, which leave when sent.
    let c = DemoGeo.composer
    let composer = Path(roundedRect: c, cornerRadius: 8, style: .continuous)
    ctx.fill(composer, with: .color(.white.opacity(0.06)))
    let focused = ramp(t, s.composerClick, s.composerClick + 0.15) * (1 - ramp(t, s.send + 0.3, s.send + 0.5))
    ctx.stroke(composer, with: .color(.white.opacity(0.10 + 0.14 * focused)), lineWidth: 0.75)
    // The placeholder is gone before the pasted words arrive, so the two never overlap.
    let pasted = ramp(t, s.fly + 0.48, s.fly + 0.63) * (1 - ramp(t, s.send, s.send + 0.12))
    let placeholder = 1 - ramp(t, s.fly + 0.28, s.fly + 0.4) * (1 - ramp(t, s.send + 0.3, s.send + 0.5))
    ctx.draw(Text("Ask anything").font(.system(size: 8.5)).foregroundColor(Palette.muted.opacity(placeholder)), at: CGPoint(x: c.minX + 10, y: c.midY), anchor: .leading)
    if pasted > 0 {
      ctx.draw(Text("This button is cut off and too…").font(.system(size: 8.5)).foregroundColor(Palette.text.opacity(pasted)),
               at: CGPoint(x: DemoGeo.composerThumb.maxX + 6, y: c.midY), anchor: .leading)
    }

    // The sent message: a bubble grows in behind the frame as it moves up, then the words.
    // At the end of the loop the whole conversation clears in place.
    let bubble = springStep(t, s.send + 0.05, 0.4)
    guard bubble > 0, reset < 1 else { return }
    ctx.drawLayer { ctx in
      ctx.opacity = 1 - reset
      if reset > 0 { ctx.addFilter(.blur(radius: 3 * reset)) }
      drawConversation(&ctx, bubble: bubble)
    }
  }

  private func drawConversation(_ ctx: inout GraphicsContext, bubble: Double) {
    let s = DemoScript.self
    let g = DemoGeo.agent
    let b = DemoGeo.bubble
    ctx.drawLayer { layer in
      layer.opacity = bubble
      let scale = 0.96 + 0.04 * bubble
      layer.translateBy(x: b.maxX, y: b.minY)
      layer.scaleBy(x: scale, y: scale)
      layer.translateBy(x: -b.maxX, y: -b.minY)
      layer.fill(Path(roundedRect: b, cornerRadius: 9, style: .continuous), with: .color(.white.opacity(0.09)))
    }
    let words = ramp(t, s.send + 0.3, s.send + 0.5)
    let transcript = ctx.resolve(Text(DemoScript.transcript).font(.system(size: 8)).foregroundColor(Palette.text.opacity(words)))
    ctx.draw(transcript, in: CGRect(x: b.minX + 6, y: DemoGeo.bubbleThumb.maxY + 4 + 3 * CGFloat(1 - words), width: b.width - 12, height: 30))
    let reply = ramp(t, s.reply, s.reply + 0.25)
    if reply > 0 {
      let text = ctx.resolve(Text(DemoScript.replyText).font(.system(size: 8)).foregroundColor(Palette.soft.opacity(reply)))
      ctx.draw(text, in: CGRect(x: g.minX + 10, y: 171 + 4 * CGFloat(1 - smooth(reply)), width: 160, height: 22))
    }
  }

  /// What the person is saying, word by word, over the bottom of the page.
  func drawSubtitle(_ ctx: inout GraphicsContext) {
    for sentence in DemoScript.sentences {
      let (a, b) = sentence.shown
      let alpha = ramp(t, a, a + 0.15) * (1 - ramp(t, b - 0.12, b))
      guard alpha > 0 else { continue }
      var line = Text("")
      for (i, (word, at)) in sentence.words.enumerated() {
        let said = 0.3 + 0.7 * ramp(t, at - 0.04, at + 0.08)
        line = line + Text((i == 0 ? "" : " ") + word).foregroundColor(.white.opacity(said))
      }
      let resolved = ctx.resolve(line.font(.system(size: 9.5, weight: .medium)))
      let size = resolved.measure(in: CGSize(width: 400, height: 40))
      let width = size.width + 32
      let center = CGPoint(x: DemoGeo.app.midX, y: 214)
      let box = CGRect(x: center.x - width / 2, y: center.y - 9, width: width, height: 18)
      ctx.drawLayer { layer in
        layer.opacity = alpha
        let scale = 0.96 + 0.04 * alpha
        layer.translateBy(x: center.x, y: center.y)
        layer.scaleBy(x: scale, y: scale)
        layer.translateBy(x: -center.x, y: -center.y)
        if alpha < 1 { layer.addFilter(.blur(radius: 3 * (1 - alpha))) }
        layer.fill(Path(roundedRect: box, cornerRadius: 9, style: .continuous), with: .color(.black.opacity(0.78)))
        var mic = layer.resolve(Image(systemName: "mic.fill"))
        mic.shading = .color(Palette.ink)
        layer.draw(mic, in: CGRect(x: box.minX + 9, y: box.midY - 4.5, width: 6, height: 9))
        layer.draw(resolved, at: CGPoint(x: box.minX + 21, y: box.midY), anchor: .leading)
      }
    }
  }

  /// The captured frame: it starts exactly over the page, then lifts off as a one-frame contact
  /// sheet (dark margin, `1` chip) and travels into the agent's chat.
  func drawFrame(_ ctx: inout GraphicsContext, in rect: CGRect) {
    let s = DemoScript.self
    let lift = springStep(t, s.finish, 0.38)
    let scale = rect.width / DemoGeo.app.width
    let margin: CGFloat = 6
    let sheet = CGRect(x: -margin, y: -margin, width: DemoGeo.app.width + 2 * margin, height: DemoGeo.app.height + 2 * margin)
    // The shadow belongs to the sheet and shrinks with it, so it never darkens the chat.
    ctx.drawLayer { layer in
      layer.addFilter(.shadow(color: .black.opacity(0.5 * lift), radius: 12 * scale, y: 5 * scale))
      layer.translateBy(x: rect.minX, y: rect.minY)
      layer.scaleBy(x: scale, y: scale)
      layer.fill(Path(roundedRect: sheet, cornerRadius: 3), with: .color(DemoInk.sheet.opacity(lift)))
    }
    ctx.drawLayer { layer in
      layer.translateBy(x: rect.minX, y: rect.minY)
      layer.scaleBy(x: scale, y: scale)
      drawPage(&layer, fixed: 0)
      layer.drawLayer { ink in
        ink.clip(to: Path(CGRect(origin: .zero, size: DemoGeo.app.size)))
        ink.stroke(DemoGeo.markPath(upTo: 1).offsetBy(dx: -DemoGeo.app.minX, dy: -DemoGeo.app.minY),
                   with: .color(Palette.ink), style: StrokeStyle(lineWidth: 3.2, lineCap: .round, lineJoin: .round))
      }
      // The keyframe's number in a small white chip with a thin dark ring, top left, like render.rs.
      let badge = ramp(t, s.finish + 0.1, s.finish + 0.3)
      if badge > 0 {
        layer.opacity = badge
        let chip = CGRect(x: 4, y: 4, width: 15, height: 15)
        layer.fill(Path(roundedRect: chip.insetBy(dx: -1.2, dy: -1.2), cornerRadius: 4.2, style: .continuous), with: .color(Color(white: 0.07)))
        layer.fill(Path(roundedRect: chip, cornerRadius: 3, style: .continuous), with: .color(Color(white: 0.98)))
        layer.draw(Text("1").font(.system(size: 10, weight: .bold)).foregroundColor(Color(white: 0.07)), at: CGPoint(x: chip.midX, y: chip.midY))
      }
    }
  }

  /// The recording pill, drawn like the real one: a black capsule that opens out of a dot,
  /// counts while recording, turns to Copied when finished and closes after the paste.
  func drawPill(_ ctx: inout GraphicsContext) {
    let s = DemoScript.self
    guard t >= s.pillOpen else { return }
    let open = springStep(t, s.pillOpen, 0.42)
    let copied = springStep(t, s.finish, 0.38)
    // Closing, the contents fade first and the capsule shrinks right after, so nothing is cut.
    let close = smooth(ramp(t, s.pillClose + 0.06, s.pillClose + 0.3))
    let contentsOut = ramp(t, s.pillClose, s.pillClose + 0.08)
    guard close < 1 else { return }
    var width = mix(DemoGeo.pillClosed, DemoGeo.pillRecording, open)
    width = mix(width, DemoGeo.pillCopied, t >= s.finish ? copied : 0)
    width = mix(width, DemoGeo.pillClosed, close)
    let alpha = ramp(t, s.pillOpen, s.pillOpen + 0.15) * (1 - close)
    let scale = (0.9 + 0.1 * open) * (1 - 0.1 * close)
    let h = DemoGeo.pillHeight
    let cx = DemoGeo.size.width / 2
    let cy = DemoGeo.pillTop + h / 2
    let shell = CGRect(x: cx - width / 2, y: DemoGeo.pillTop, width: width, height: h)
    let capsule = Path(roundedRect: shell, cornerRadius: h / 2, style: .continuous)

    ctx.drawLayer { layer in
      layer.opacity = alpha
      layer.translateBy(x: cx, y: cy)
      layer.scaleBy(x: scale, y: scale)
      layer.translateBy(x: -cx, y: -cy)
      layer.drawLayer { shadow in
        shadow.addFilter(.shadow(color: .black.opacity(0.4), radius: 7, y: 2.5))
        shadow.fill(capsule, with: .color(DemoInk.pill))
      }
      let border = t >= s.finish ? copied : 0
      layer.stroke(Path(roundedRect: shell.insetBy(dx: 0.5, dy: 0.5), cornerRadius: h / 2 - 0.5, style: .continuous),
                   with: .color(Color.white.opacity(0.12 * (1 - border)).mix(Palette.positive.opacity(0.34), border)), lineWidth: 1)
      layer.clip(to: capsule)

      let out = t >= s.finish ? ramp(t, s.finish, s.finish + 0.12) : 0
      let into = t >= s.finish ? ramp(t, s.finish + 0.04, s.finish + 0.22) : 0
      if out < 1 {
        // Opening, the contents come in once the capsule is nearly wide enough for them.
        drawPillContent(&layer, copied: false, center: cx, alpha: (1 - out) * ramp(t, s.pillOpen + 0.14, s.pillOpen + 0.3))
      }
      if into > 0 && contentsOut < 1 {
        drawPillContent(&layer, copied: true, center: cx, alpha: into * (1 - contentsOut))
      }
    }
  }

  private func drawPillContent(_ ctx: inout GraphicsContext, copied: Bool, center: CGFloat, alpha: Double) {
    let s = DemoScript.self
    let full = copied ? DemoGeo.pillCopied : DemoGeo.pillRecording
    let x0 = center - full / 2
    let cy = DemoGeo.pillTop + DemoGeo.pillHeight / 2
    ctx.drawLayer { layer in
      layer.opacity = alpha
      if alpha < 1 {
        let k = 0.96 + 0.04 * alpha
        layer.translateBy(x: center, y: cy)
        layer.scaleBy(x: k, y: k)
        layer.translateBy(x: -center, y: -cy)
        layer.addFilter(.blur(radius: 3 * (1 - alpha)))
      }
      let slot = CGPoint(x: x0 + 3 + 12, y: cy)
      if copied {
        layer.fill(Path(ellipseIn: CGRect(x: slot.x - 7, y: slot.y - 7, width: 14, height: 14)), with: .color(Palette.positive.opacity(0.18)))
        var check = Path()
        check.move(to: CGPoint(x: slot.x - 3.2, y: slot.y + 0.2))
        check.addLine(to: CGPoint(x: slot.x - 0.9, y: slot.y + 2.5))
        check.addLine(to: CGPoint(x: slot.x + 3.4, y: slot.y - 2.4))
        layer.stroke(check, with: .color(Palette.positive), style: StrokeStyle(lineWidth: 1.4, lineCap: .round, lineJoin: .round))
        layer.draw(Text("Copied").font(.system(size: 10, weight: .semibold)).foregroundColor(Palette.text), at: CGPoint(x: x0 + 27, y: cy), anchor: .leading)
      } else {
        let beat = (t - s.recordStart).truncatingRemainder(dividingBy: 1.6) / 1.6
        let halo = 7.5 * (0.4 + 0.6 * beat)
        layer.fill(Path(ellipseIn: CGRect(x: slot.x - halo, y: slot.y - halo, width: halo * 2, height: halo * 2)),
                   with: .color(Palette.ink.opacity(0.34 * (1 - beat))))
        layer.fill(Path(ellipseIn: CGRect(x: slot.x - 3, y: slot.y - 3, width: 6, height: 6)), with: .color(Palette.ink))
        let seconds = max(0, Int(min(t, s.finish) - s.recordStart))
        layer.draw(Text("0:0\(seconds)").font(.system(size: 10, weight: .semibold).monospacedDigit()).foregroundColor(Palette.text),
                   at: CGPoint(x: x0 + 27, y: cy), anchor: .leading)
      }
      let dividerX = x0 + 3 + 24 + (copied ? 38 : 24) + 6 + 4.5
      layer.fill(Path(CGRect(x: dividerX, y: cy - 6, width: 1, height: 12)), with: .color(.white.opacity(0.1)))
      let chipWidth: CGFloat = copied ? 44 : 46
      let pressedAt = copied ? s.pasteKeys : s.finishKeys
      let press = (pressedAt..<pressedAt + 0.18).contains(t) ? 0.92 : 1.0
      let chip = CGRect(x: x0 + full - 3 - chipWidth, y: cy - 12, width: chipWidth, height: 24)
      layer.drawLayer { c in
        c.translateBy(x: chip.midX, y: chip.midY)
        c.scaleBy(x: press, y: press)
        c.translateBy(x: -chip.midX, y: -chip.midY)
        c.fill(Path(roundedRect: chip, cornerRadius: 12, style: .continuous), with: .color(DemoInk.accent))
        c.draw(Text(copied ? "Paste" : "Finish").font(.system(size: 9.5, weight: .semibold)).foregroundColor(Palette.text), at: CGPoint(x: chip.midX, y: chip.midY))
      }
    }
  }
}

extension Color {
  /// `self` blended toward `other` by `amount` (0…1), in sRGB.
  func mix(_ other: Color, _ amount: Double) -> Color {
    let a = NSColor(self).usingColorSpace(.sRGB) ?? .clear
    let b = NSColor(other).usingColorSpace(.sRGB) ?? .clear
    let p = CGFloat(clamp01(amount))
    return Color(.sRGB,
                 red: Double(a.redComponent + (b.redComponent - a.redComponent) * p),
                 green: Double(a.greenComponent + (b.greenComponent - a.greenComponent) * p),
                 blue: Double(a.blueComponent + (b.blueComponent - a.blueComponent) * p),
                 opacity: Double(a.alphaComponent + (b.alphaComponent - a.alphaComponent) * p))
  }
}

/// The macOS arrow, tip at the origin.
struct DemoCursor: Shape {
  func path(in _: CGRect) -> Path {
    var p = Path()
    p.move(to: .zero)
    p.addLine(to: CGPoint(x: 0, y: 15.5))
    p.addLine(to: CGPoint(x: 3.7, y: 12))
    p.addLine(to: CGPoint(x: 6.3, y: 17.8))
    p.addLine(to: CGPoint(x: 8.8, y: 16.7))
    p.addLine(to: CGPoint(x: 6.3, y: 11.1))
    p.addLine(to: CGPoint(x: 11.3, y: 10.9))
    p.closeSubpath()
    return p
  }
}

/// The whole stage at time `t`: the scene, then the pretend pointer on top.
struct DemoStageFrame: View {
  let t: Double
  var showCursor = true

  var body: some View {
    let scene = DemoScene(t: t)
    let cursor = scene.cursor
    ZStack(alignment: .topLeading) {
      Canvas { ctx, _ in scene.draw(&ctx) }
      DemoCursor()
        .fill(Color.black)
        .overlay(DemoCursor().stroke(Color.white, style: StrokeStyle(lineWidth: 1.2, lineJoin: .round)))
        .frame(width: 12, height: 18)
        .scaleEffect(cursor.pressed ? 0.88 : 1, anchor: .topLeading)
        .shadow(color: .black.opacity(0.35), radius: 1.5, y: 1)
        .offset(x: cursor.point.x, y: cursor.point.y)
        .opacity(showCursor ? 1 : 0)
        .animation(.easeOut(duration: 0.15), value: showCursor)
    }
    .frame(width: DemoGeo.size.width, height: DemoGeo.size.height, alignment: .topLeading)
  }
}

// MARK: Captions

enum CaptionPart {
  case text(String)
  case keys(String, lit: ClosedRange<Double>)
  case symbol(String)
}

struct DemoBeat {
  let start: Double
  let parts: [CaptionPart]
}

func demoBeats(shortcut: String, paste: String) -> [DemoBeat] {
  let s = DemoScript.self
  let start = DemoBeat(start: 0, parts: [.text("Tap"), .keys(shortcut, lit: s.startKeys...(s.startKeys + 0.35)), .text("to start recording")])
  return [
    start,
    DemoBeat(start: 1.1, parts: [.symbol("mic.fill"), .text("Talk as you go")]),
    DemoBeat(start: 2.4, parts: [.text("Hold"), .keys("⇧", lit: 2.55...3.95), .text("and move the mouse to mark")]),
    DemoBeat(start: 4.2, parts: [.symbol("cursorarrow.click"), .text("Click to point at something")]),
    DemoBeat(start: 5.5, parts: [.text("Tap"), .keys(shortcut, lit: s.finishKeys...(s.finishKeys + 0.3)), .text("again to finish")]),
    DemoBeat(start: 6.6, parts: [.text("Press"), .keys(paste, lit: s.pasteKeys...(s.pasteKeys + 0.3)), .text("to paste into your agent")]),
    DemoBeat(start: 8.2, parts: [.text("Your agent sees your mark and reads what you said")]),
    DemoBeat(start: s.resetCaption, parts: start.parts),
  ]
}

struct CaptionLine: View {
  let beat: DemoBeat
  let t: Double

  var body: some View {
    HStack(spacing: 7) {
      ForEach(Array(beat.parts.enumerated()), id: \.offset) { _, part in
        switch part {
        case let .text(text):
          Text(text).font(.system(size: 13.5)).foregroundStyle(Palette.soft)
        case let .symbol(name):
          Image(systemName: name).font(.system(size: 12, weight: .medium)).foregroundStyle(Palette.muted)
        case let .keys(keys, lit):
          let on = lit.contains(t)
          Keycaps(shortcut: keys, litMods: on, size: 22)
            .overlay {
              // A single key (⇧) lights as a whole.
              if keys.count == 1 && on {
                RoundedRectangle(cornerRadius: 22 * 0.24, style: .continuous).strokeBorder(Palette.ink.opacity(0.7), lineWidth: 1)
              }
            }
            .scaleEffect(on && t - lit.lowerBound < 0.12 ? 0.94 : 1)
        }
      }
    }
    .fixedSize()
  }
}

/// The current step's caption; a new one slides up from under the row's lower edge.
struct DemoCaptions: View {
  let beats: [DemoBeat]
  let t: Double

  var body: some View {
    let index = beats.lastIndex { $0.start <= t } ?? 0
    let beat = beats[index]
    let p = index == 0 ? 1 : springStep(t, beat.start, 0.35)
    ZStack {
      if p < 1, index > 0 {
        CaptionLine(beat: beats[index - 1], t: t)
          .offset(y: -16 * CGFloat(p))
          .opacity(1 - p)
      }
      CaptionLine(beat: beat, t: t)
        .offset(y: 16 * CGFloat(1 - p))
        .opacity(p)
    }
    .frame(maxWidth: .infinity)
    .frame(height: 28)
    .mask(LinearGradient(stops: [
      .init(color: .clear, location: 0), .init(color: .black, location: 0.18),
      .init(color: .black, location: 0.82), .init(color: .clear, location: 1),
    ], startPoint: .top, endPoint: .bottom))
  }
}

// MARK: The screen

/// Lets a render show a mark drawn on the demo.
var onboardingDemoPractice: (strokes: [[CGPoint]], drawing: Bool, done: Bool)?
/// Renders the reduced-motion storyboard whatever the system setting.
var onboardingDemoStoryboard = false

/// Watches ⇧ while the window has the keyboard.
final class ShiftWatcher: ObservableObject {
  @Published var down = NSEvent.modifierFlags.contains(.shift)
  private var monitor: Any?

  func start() {
    guard monitor == nil else { return }
    monitor = NSEvent.addLocalMonitorForEvents(matching: .flagsChanged) { [weak self] event in
      self?.down = event.modifierFlags.contains(.shift)
      return event
    }
  }

  func stop() {
    if let monitor { NSEvent.removeMonitor(monitor) }
    monitor = nil
  }
}

struct HowItWorksDemo: View {
  let shortcut: String
  let paste: String
  /// Rust's record of the demo (`watched`, `practiced`, `skipped`), nil before anything.
  let tutorial: String?
  let send: (String) -> Void

  @State private var base: Double = 0
  @State private var since: Date? = Date()
  @State private var idleRevision = 0
  @State private var strokes: [[CGPoint]] = onboardingDemoPractice?.strokes ?? []
  @State private var drawing = onboardingDemoPractice?.drawing ?? false
  @State private var practiced = onboardingDemoPractice?.done ?? false
  @State private var inkOpacity = 1.0
  @State private var resume: DispatchWorkItem?
  @State private var reportedWatched = false
  /// The moment a rewind left, fading out over the one it went to.
  @State private var ghost: Double?
  @State private var ghostOpacity = 0.0
  @StateObject private var shift = ShiftWatcher()
  private let ticker = Timer.publish(every: 0.5, on: .main, in: .common).autoconnect()

  private var storyboard: Bool { reduceMotion || onboardingDemoStoryboard }
  private var beats: [DemoBeat] { demoBeats(shortcut: shortcut, paste: paste) }
  private var paused: Bool { since == nil }

  private func elapsed(_ date: Date) -> Double {
    base + (since.map { date.timeIntervalSince($0) } ?? 0)
  }

  var body: some View {
    VStack(spacing: 0) {
      if storyboard {
        DemoStoryboard(shortcut: shortcut, paste: paste)
          .overlay(practiceLayer)
      } else if let phase = onboardingFixedPhase {
        stage(t: phase * DemoScript.loop)
          .overlay(alignment: .top) { stageOverlay }
      } else {
        // Re-made whenever it pauses or resumes (`idleRevision`), so the pause always takes.
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: paused)) { context in
          stage(t: elapsed(context.date).truncatingRemainder(dividingBy: DemoScript.loop))
        }
        .id(idleRevision)
        .overlay(alignment: .top) { rewindGhost }
        .overlay(alignment: .top) { stageOverlay }
      }
      if storyboard {
        hint.frame(height: 16).padding(.top, 8)
      }
    }
    .onChange(of: shift.down) { _, down in if !down { endStroke() } }
    .onReceive(ticker) { _ in reportWatched() }
    .onAppear {
      guard onboardingFixedPhase == nil else { return }
      shift.start()
      since = Date()
      if storyboard { reportWatched(force: true) }
    }
    .onDisappear {
      shift.stop()
      resume?.cancel()
    }
  }

  private func stage(t: Double) -> some View {
    VStack(spacing: 12) {
      ZStack {
        RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color(red: 0.10, green: 0.10, blue: 0.105))
        DemoStageFrame(t: t, showCursor: strokes.isEmpty && !drawing)
      }
      .frame(width: DemoGeo.size.width, height: DemoGeo.size.height)
      .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
      .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Palette.line, lineWidth: 1))
      // Drawing on the stage holds the ⇧ caption, lit while ⇧ is down.
      let practicing = drawing || !strokes.isEmpty
      ZStack {
        DemoCaptions(beats: beats, t: practicing ? (drawing ? 3.0 : 3.98) : t)
          .id(practicing)
          .transition(Motion.swap)
      }
      .animation(Motion.state, value: practicing)
    }
  }

  // MARK: Trying a mark

  /// Over the stage and outside the clock, so a stroke isn't interrupted when the loop pauses:
  /// the ink being drawn, and the hint where the hand goes, on the page's left edge.
  private var stageOverlay: some View {
    ZStack(alignment: .topLeading) {
      practiceLayer
      hint
        .frame(height: DemoGeo.pillHeight)
        .padding(.leading, DemoGeo.app.minX)
        .padding(.top, DemoGeo.pillTop)
        .allowsHitTesting(false)
    }
    .frame(width: DemoGeo.size.width, height: DemoGeo.size.height)
  }

  private var practiceLayer: some View {
    Canvas { ctx, _ in
      for stroke in strokes where stroke.count > 1 {
        var path = Path()
        path.addLines(stroke)
        ctx.stroke(path, with: .color(Palette.ink), style: StrokeStyle(lineWidth: 3.2, lineCap: .round, lineJoin: .round))
      }
    }
    .opacity(inkOpacity)
    .contentShape(Rectangle())
    .onContinuousHover(coordinateSpace: .local) { phase in
      switch phase {
      case let .active(point):
        if NSEvent.modifierFlags.contains(.shift) {
          draw(to: point)
        } else {
          endStroke()
        }
      case .ended:
        endStroke()
      }
    }
  }

  private func draw(to point: CGPoint) {
    resume?.cancel()
    if !drawing {
      if inkOpacity < 1 { strokes = [] }
      inkOpacity = 1
      drawing = true
      strokes.append([point])
      pause()
      return
    }
    guard let last = strokes.last?.last, hypot(point.x - last.x, point.y - last.y) >= 1.5 else { return }
    strokes[strokes.count - 1].append(point)
  }

  private func endStroke() {
    guard drawing else { return }
    drawing = false
    let stroke = strokes.last ?? []
    let length = zip(stroke, stroke.dropFirst()).reduce(0) { $0 + hypot($1.1.x - $1.0.x, $1.1.y - $1.0.y) }
    if length >= 40 {
      if !practiced { withAnimation(Motion.state) { practiced = true } }
      if tutorial != "practiced" { send("practiced") }
    }
    // The mark stays a moment, fades, and the demo carries on from where it stopped.
    let work = DispatchWorkItem {
      withAnimation(.easeOut(duration: 0.25)) { inkOpacity = 0 }
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.27) {
        guard !drawing else { return }
        strokes = []
        inkOpacity = 1
        if !storyboard && since == nil {
          since = Date()
          idleRevision += 1
        }
      }
    }
    resume = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 2.2, execute: work)
  }

  /// Stops the loop for a mark. While the first words show and nothing is drawn yet it stops
  /// where it is; anywhere else it goes back to that moment (2.1 s: recording, "This button is
  /// cut off"), softly, so the mark goes on the page rather than on the lifted frame or the chat.
  private func pause() {
    guard let since, !storyboard else { return }
    let total = base + Date().timeIntervalSince(since)
    let t = total.truncatingRemainder(dividingBy: DemoScript.loop)
    let ready = (1.4..<2.35).contains(t)
    if ready {
      base = total
      self.since = nil
      idleRevision += 1
    } else {
      base = total - t + 2.1
      self.since = nil
      idleRevision += 1
      // Only the old moment fades, on top of the new one, so the page both share never dims.
      ghost = t
      ghostOpacity = 1
      DispatchQueue.main.async {
        withAnimation(.easeOut(duration: 0.22)) { ghostOpacity = 0 }
      }
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { ghost = nil }
    }
  }

  @ViewBuilder private var rewindGhost: some View {
    if let ghost {
      DemoStageFrame(t: ghost, showCursor: false)
        .blur(radius: 3 * (1 - ghostOpacity))
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .opacity(ghostOpacity)
        .allowsHitTesting(false)
    }
  }

  private func reportWatched(force: Bool = false) {
    guard !reportedWatched, tutorial != "watched", tutorial != "practiced" else { return }
    guard force || elapsed(Date()) >= DemoScript.end else { return }
    reportedWatched = true
    send("watched")
  }

  private var hint: some View {
    let text: String = if drawing {
      "Let go of ⇧ to finish"
    } else if practiced && !strokes.isEmpty {
      "That's a mark"
    } else if storyboard {
      "Try it: hold ⇧ over the pictures and move the mouse"
    } else {
      "Try it: hold ⇧ and move"
    }
    return ZStack(alignment: .leading) {
      Text(text)
        .font(.system(size: storyboard ? 11.5 : 11))
        .foregroundStyle(text == "That's a mark" ? Palette.soft : Palette.muted)
        .id(text)
        .transition(Motion.swap)
    }
    .animation(Motion.state, value: text)
  }
}

/// Reduced motion: four still frames of the same story, each with its caption.
struct DemoStoryboard: View {
  let shortcut: String
  let paste: String

  var body: some View {
    let frames: [(Double, String)] = [
      (3.95, "Tap \(shortcut), talk, and hold ⇧ to mark"),
      (5.0, "Click to point at something"),
      (6.5, "Tap \(shortcut) again to finish"),
      (9.4, "Press \(paste) to paste into your agent"),
    ]
    let scale: CGFloat = 0.44
    let cell = CGSize(width: DemoGeo.size.width * scale, height: DemoGeo.size.height * scale)
    return LazyVGrid(columns: [GridItem(.fixed(cell.width), spacing: 10), GridItem(.fixed(cell.width), spacing: 10)], spacing: 6) {
      ForEach(Array(frames.enumerated()), id: \.offset) { _, frame in
        VStack(spacing: 5) {
          DemoStageFrame(t: frame.0)
            .scaleEffect(scale, anchor: .topLeading)
            .frame(width: cell.width, height: cell.height, alignment: .topLeading)
            .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).strokeBorder(Palette.line, lineWidth: 1))
          Text(frame.1)
            .font(.system(size: 11))
            .foregroundStyle(Palette.soft)
            .lineLimit(1)
        }
      }
    }
    .frame(width: DemoGeo.size.width)
  }
}

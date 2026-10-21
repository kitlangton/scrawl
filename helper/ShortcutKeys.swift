import AppKit
import SwiftUI

// The shortcut control in Settings: only the keys of the shortcut, as rounded keycaps with a
// little depth, in a recessed field (Hex's control, Scrawl's red).
//
//   Resting      ⌥ ⇧ S in the field. Holding the shortcut's modifiers presses those keys onto
//                their bases and turns the key red: pressing it now would fire.
//   Listening    The keys leave, the field tints red, a dot breathes and "Type a shortcut"
//                waits. Modifiers land as they go down, in macOS order, and lift out when let go;
//                their neighbours make room first. A red caret marks where the key goes.
//   Accepted     The new keys bob once, left to right.
//   Refused      The pressed keys stay a moment in red and shake, then the old ones come back
//                and the caption says why.
//   Off          No keys: "Set shortcut" sits in the field (paste only).
//
// Every value is a spring evaluated in closed form at a time, so a moment draws the same live
// from a TimelineView or frame by frame for `scrawl-helper settings --render`. Retargeting
// keeps position and velocity. Timings (s): enter .3 (after .06 when others move), exit .22,
// slide .32, width .28, press down .07 and up .24, tint .2; refused keys turn red at +.08 and shake from +.1 for .55,
// old keys at +.7; accepted bob over .5. Reduced motion: no travel, shake or press depth;
// presence and colour fade in .15.

// MARK: Springs

/// A scalar on a spring that can be retargeted mid-flight, keeping position and velocity.
/// Closed form, so any moment can be evaluated live or frame by frame.
struct Track {
  var spring: Spring
  var from: Double
  var to: Double
  var v0 = 0.0
  var t0 = -1e9

  init(_ value: Double, _ spring: Spring) {
    from = value
    to = value
    self.spring = spring
  }

  func value(_ t: Double) -> Double {
    let dt = t - t0
    if dt <= 0 { return from }
    if dt >= spring.settlingDuration { return to }
    return spring.value(fromValue: from, toValue: to, initialVelocity: v0, time: dt)
  }

  func velocity(_ t: Double) -> Double {
    let dt = t - t0
    if dt <= 0 || dt >= spring.settlingDuration { return 0 }
    return spring.velocity(fromValue: from, toValue: to, initialVelocity: v0, time: dt)
  }

  mutating func set(_ target: Double, at t: Double, _ s: Spring? = nil) {
    guard target != to else { return }
    let x = value(t), v = velocity(t)
    from = x
    v0 = v
    to = target
    t0 = t
    if let s { spring = s }
  }

  mutating func jump(_ v: Double, at t: Double) {
    from = v
    to = v
    v0 = 0
    t0 = t
  }

  func still(_ t: Double) -> Bool { t - t0 >= spring.settlingDuration }
}

enum KeySprings {
  static let enter = Spring(duration: 0.3, bounce: 0)
  static let exit = Spring(duration: 0.22, bounce: 0)
  static let slide = Spring(duration: 0.32, bounce: 0)
  static let width = Spring(duration: 0.28, bounce: 0)
  static let down = Spring(duration: 0.07, bounce: 0)
  static let up = Spring(duration: 0.24, bounce: 0)
  static let tint = Spring(duration: 0.2, bounce: 0)
  static let draw = Spring(duration: 0.5, bounce: 0)
  static let fade = Spring(duration: 0.35, bounce: 0)
  static let reduced = Spring(duration: 0.15, bounce: 0)
}

struct KeyMetrics {
  /// A key's height, its depth included.
  var height: CGFloat
  /// The narrowest key (one glyph).
  var unit: CGFloat
  var gap: CGFloat
  /// Inside the container, each side.
  var pad: CGFloat
  /// Room before the keys while listening (the red dot).
  var lead: CGFloat
  /// Room after the last key while listening (the caret).
  var slot: CGFloat
  /// The content's width while listening with nothing held (the prompt).
  var empty: CGFloat
  /// Key labels.
  var font: CGFloat

  func width(_ label: String) -> CGFloat {
    label.count == 1 ? unit : max(unit, CGFloat(label.count) * font * 0.6 + 14)
  }
}

// MARK: Motion

/// The keys on screen, each with its own identity (its label): entering where it will rest,
/// leaving where it was, sliding when neighbours come and go, pressing when held. Positions are
/// key centres measured from the container's trailing edge.
final class KeyMotion {
  struct Key {
    var label: String
    var x: Track
    var presence: Track
    var press: Track
    var ink: Track
  }

  let m: KeyMetrics
  let reduce: Bool
  private(set) var keys: [String: Key] = [:]
  private(set) var order: [String] = []
  var width: Track
  var listen = Track(0, KeySprings.enter)
  /// 1 while there are no keys and nothing listens: the "Set shortcut" placeholder.
  var blank = Track(0, KeySprings.enter)
  var caret = Track(0, KeySprings.slide)
  /// 0…1…0 after a new shortcut lands.
  var flourish = Track(0, KeySprings.draw)
  var flourishFade = Track(1, KeySprings.fade)
  var shakeAt = -10.0
  private var tapUntil: [String: Double] = [:]
  private var pending: [(at: Double, run: () -> Void)] = []

  init(_ m: KeyMetrics, reduce: Bool) {
    self.m = m
    self.reduce = reduce
    width = Track(0, KeySprings.width)
  }

  private func spring(_ s: Spring) -> Spring { reduce ? KeySprings.reduced : s }

  private func move(_ track: inout Track, _ target: Double, at t: Double, instant: Bool) {
    if instant || reduce { track.jump(target, at: t) } else { track.set(target, at: t) }
  }

  func show(_ labels: [String], at t: Double, instant: Bool = false) {
    let listening = listen.to > 0.5
    var x = m.pad + (listening ? m.lead : 0)
    var centres: [String: CGFloat] = [:]
    for (i, label) in labels.enumerated() {
      let w = m.width(label)
      if i > 0 { x += m.gap }
      centres[label] = x + w / 2
      x += w
    }
    var caretCentre = x
    let blankNow = !listening && labels.isEmpty
    if blankNow { x += m.empty }
    move(&blank, blankNow ? 1 : 0, at: t, instant: instant)
    if listening {
      if labels.isEmpty {
        caretCentre = x + m.empty / 2
        x += m.empty
      } else {
        caretCentre = x + m.gap + m.slot / 2
        x += m.gap + m.slot
      }
    }
    let total = x + m.pad
    move(&width, total, at: t, instant: instant)
    if listening {
      let opened = listen.from < 0.5 && listen.value(t) < 0.05
      move(&caret, caretCentre - total, at: t, instant: instant || opened)
    }
    for (label, centre) in centres {
      let target = centre - total
      if var key = keys[label], key.presence.to > 0.5 || key.presence.value(t) > 0.02 {
        move(&key.x, target, at: t, instant: instant)
        key.presence.set(1, at: t, spring(KeySprings.enter))
        keys[label] = key
      } else {
        var key = Key(
          label: label,
          x: Track(target, KeySprings.slide),
          presence: Track(instant ? 1 : 0, spring(KeySprings.enter)),
          press: Track(0, KeySprings.up),
          ink: Track(0, spring(KeySprings.tint))
        )
        // Its neighbours start making room first, so it lands in a gap rather than on them.
        let crowded = !instant && centres.count > 1
        key.presence.set(1, at: crowded ? t + 0.06 : t)
        keys[label] = key
        order.removeAll { $0 == label }
        order.append(label)
      }
    }
    for label in keys.keys where centres[label] == nil {
      keys[label]?.presence.set(0, at: t, spring(KeySprings.exit))
    }
  }

  func press(_ labels: Set<String>, at t: Double) {
    for label in keys.keys where t >= (tapUntil[label] ?? -1) {
      let down = labels.contains(label)
      keys[label]?.press.set(down ? 1 : 0, at: t, down ? KeySprings.down : KeySprings.up)
    }
  }

  func tap(_ label: String, at t: Double) {
    keys[label]?.press.set(1, at: t, KeySprings.down)
    tapUntil[label] = t + 0.14
    later(t + 0.14) { [weak self] in self?.keys[label]?.press.set(0, at: t + 0.14, KeySprings.up) }
  }

  func ink(_ labels: Set<String>, at t: Double) {
    for label in keys.keys {
      keys[label]?.ink.set(labels.contains(label) ? 1 : 0, at: t)
    }
  }

  func flourish(at t: Double) {
    flourish.jump(0, at: t)
    flourishFade.jump(1, at: t)
    if reduce {
      flourish.jump(1, at: t)
      flourishFade.jump(0, at: t)
      flourishFade.set(1, at: t, KeySprings.reduced)
    } else {
      later(t + 0.05) { [weak self] in self?.flourish.set(1, at: t + 0.05) }
    }
    later(t + 1.0) { [weak self] in self?.flourishFade.set(0, at: t + 1.0) }
    later(t + 1.4) { [weak self] in
      self?.flourish.jump(0, at: t + 1.4)
      self?.flourishFade.jump(1, at: t + 1.4)
    }
  }

  func later(_ at: Double, _ run: @escaping () -> Void) {
    pending.append((at, run))
  }

  func advance(to t: Double) {
    while let i = pending.indices.min(by: { pending[$0].at < pending[$1].at }), pending[i].at <= t {
      pending.remove(at: i).run()
    }
  }

  func shake(_ t: Double) -> CGFloat {
    let dt = t - shakeAt
    guard !reduce, dt >= 0, dt < 0.55 else { return 0 }
    return CGFloat(5 * exp(-dt * 8) * sin(2 * .pi * 7 * dt))
  }

  func still(_ t: Double) -> Bool {
    pending.isEmpty && listen.to == 0 && listen.still(t) && blank.still(t) && width.still(t) && caret.still(t)
      && flourish.still(t) && flourishFade.still(t) && t - shakeAt >= 0.55
      && keys.values.allSatisfy { $0.x.still(t) && $0.presence.still(t) && $0.press.still(t) && $0.ink.still(t) }
  }

  func prune(_ t: Double) {
    for (label, key) in keys where key.presence.to == 0 && key.presence.still(t) {
      keys[label] = nil
      order.removeAll { $0 == label }
    }
  }
}

let keyModifiers = ["⌃", "⌥", "⇧", "⌘"]

/// The keys of a shortcut in macOS order: ⌃ ⌥ ⇧ ⌘, then the key.
func keyLabels(_ s: SettingsShortcut) -> [String] {
  var out: [String] = []
  if s.control { out.append("⌃") }
  if s.option { out.append("⌥") }
  if s.shift { out.append("⇧") }
  if s.command { out.append("⌘") }
  if !s.key.isEmpty { out.append(s.key) }
  return out
}

func keyLabels(_ flags: NSEvent.ModifierFlags) -> Set<String> {
  var out: Set<String> = []
  if flags.contains(.control) { out.insert("⌃") }
  if flags.contains(.option) { out.insert("⌥") }
  if flags.contains(.shift) { out.insert("⇧") }
  if flags.contains(.command) { out.insert("⌘") }
  return out
}

// MARK: Controller

/// Decides what one shortcut's keys show, from what Scrawl sent and what the hands are doing.
final class KeyController {
  let motion: KeyMotion
  private(set) var shortcut: [String]
  private(set) var recording = false
  private var held: Set<String> = []
  private var typed: String?
  private var refusing = false
  private var refusedAt = -10.0
  private var attempt: [String] = []

  init(_ m: KeyMetrics, shortcut: [String], reduce: Bool = false, at t: Double) {
    motion = KeyMotion(m, reduce: reduce)
    self.shortcut = shortcut
    motion.show(shortcut, at: t, instant: true)
  }

  func update(shortcut new: [String], recording on: Bool, error: String?, at t: Double) {
    let wasRecording = recording
    recording = on
    if wasRecording, !on, error != nil, !attempt.isEmpty {
      refuse(restoring: new, at: t)
    }
    if !on { typed = nil }
    if on, !wasRecording {
      attempt = []
      refusing = false
      motion.listen.set(1, at: t)
    } else if !on, wasRecording {
      motion.listen.set(0, at: t)
    }
    if !refusing {
      if new != shortcut, !on { motion.flourish(at: t) }
      shortcut = new
    }
    refresh(at: t)
  }

  func hold(_ mods: Set<String>, at t: Double) {
    guard mods != held else { return }
    held = mods
    if recording { typed = nil }
    refresh(at: t)
  }

  func type(_ key: String, at t: Double) {
    guard recording else { return }
    typed = key
    attempt = keyModifiers.filter(held.contains) + [key]
    refresh(at: t)
    motion.tap(key, at: t)
  }

  private func refuse(restoring old: [String], at t: Double) {
    refusing = true
    refusedAt = t
    let m = motion
    m.shakeAt = t + 0.1
    m.later(t + 0.7) { [weak self] in
      guard let self else { return }
      refusing = false
      attempt = []
      shortcut = old
      refresh(at: t + 0.7)
    }
  }

  private func refresh(at t: Double) {
    motion.prune(t)
    if refusing {
      motion.show(attempt, at: t)
      // Red once the refused key has landed.
      motion.ink(Set(attempt), at: max(t, refusedAt + 0.08))
      motion.press([], at: t)
    } else if recording {
      motion.show(keyModifiers.filter(held.contains) + (typed.map { [$0] } ?? []), at: t)
      motion.ink([], at: t)
      motion.press(held, at: t)
    } else {
      motion.show(shortcut, at: t)
      let mods = Set(shortcut.filter(keyModifiers.contains))
      motion.press(held.intersection(mods), at: t)
      let ready = !mods.isEmpty && mods.isSubset(of: held)
      motion.ink(ready ? Set(shortcut.filter { !keyModifiers.contains($0) }) : [], at: t)
    }
  }
}

// MARK: Views

let keyInk = Color(red: 1, green: 0x3B / 255, blue: 0x30 / 255)

private func mix(_ a: Color, _ b: Color, _ t: Double) -> Color {
  guard let a = NSColor(a).usingColorSpace(.sRGB), let b = NSColor(b).usingColorSpace(.sRGB) else { return Color(a) }
  let t = max(0, min(1, t))
  return Color(
    .sRGB,
    red: a.redComponent + (b.redComponent - a.redComponent) * t,
    green: a.greenComponent + (b.greenComponent - a.greenComponent) * t,
    blue: a.blueComponent + (b.blueComponent - a.blueComponent) * t,
    opacity: a.alphaComponent + (b.alphaComponent - a.alphaComponent) * t
  )
}

// MARK: Keycap

/// A rounded-rect key standing on its own base: the base shows below the cap as depth, and the
/// cap sinks onto it when pressed.
struct ShortcutKeycap: View {
  let label: String
  let width: CGFloat
  let height: CGFloat
  var depth: CGFloat = 2
  var radius: CGFloat = 6
  var font: CGFloat = 13
  let press: Double
  var ink = 0.0
  let dark: Bool

  var body: some View {
    let down = CGFloat(press) * (depth - 0.5)
    let face = height - depth
    let top = dark ? Color(red: 0.205, green: 0.205, blue: 0.215) : Color.white
    let bottom = dark ? Color(red: 0.165, green: 0.165, blue: 0.175) : Color(red: 0.965, green: 0.965, blue: 0.97)
    let base = dark ? Color(red: 0.06, green: 0.06, blue: 0.065) : Color(red: 0.78, green: 0.78, blue: 0.8)
    let edge = dark ? Color.white.opacity(0.07) : Color.black.opacity(0.11)
    let glyph = dark ? Color.white.opacity(0.86 + 0.14 * press) : Color.black.opacity(0.8 + 0.15 * press)
    ZStack(alignment: .top) {
      RoundedRectangle(cornerRadius: radius, style: .continuous)
        .fill(base)
        .frame(width: width, height: face)
        .offset(y: depth)
      RoundedRectangle(cornerRadius: radius, style: .continuous)
        .fill(LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom))
        .overlay(
          RoundedRectangle(cornerRadius: radius, style: .continuous)
            .strokeBorder(mix(edge, keyInk.opacity(0.75), ink), lineWidth: 1)
        )
        .overlay(alignment: .top) {
          // A catch of light along the top edge, as on a real cap.
          RoundedRectangle(cornerRadius: radius, style: .continuous)
            .strokeBorder(
              LinearGradient(colors: [.white.opacity(dark ? 0.1 : 0.9), .clear], startPoint: .top, endPoint: .center),
              lineWidth: 1
            )
            .padding(0.5)
        }
        .overlay(
          Text(label)
            .font(.system(size: label.count > 1 ? font - 2 : font, weight: .medium))
            .foregroundStyle(mix(glyph, keyInk, ink))
        )
        .frame(width: width, height: face)
        .offset(y: down)
    }
    .frame(width: width, height: height, alignment: .top)
    .shadow(color: .black.opacity(dark ? 0.3 : 0.07), radius: 1, y: 1)
  }
}

/// The keys of one shortcut, drawn from `KeyMotion` at `t`, inside a frame of the motion's width
/// with the keys' trailing edge at its trailing edge.
struct KeyRow: View {
  let k: KeyMotion
  let t: Double
  let dark: Bool
  var depth: CGFloat = 2
  var radius: CGFloat = 6
  /// Each key bobs once, left to right, as a new shortcut lands.
  var ripple = false

  var body: some View {
    let w = CGFloat(k.width.value(t))
    let f = k.flourish.value(t)
    ZStack(alignment: .topLeading) {
      ForEach(Array(k.order.enumerated()), id: \.element) { _, label in
        if let key = k.keys[label] {
          let p = key.presence.value(t)
          if p > 0.003 {
            let kw = k.m.width(label)
            let i = Double(k.order.filter { k.keys[$0]?.presence.to == 1 }.firstIndex(of: label) ?? 0)
            let bob = ripple ? max(0, sin(.pi * min(1, max(0, (f - i * 0.14) / 0.42)))) * 0.85 : 0
            ShortcutKeycap(label: label, width: kw, height: k.m.height, depth: depth, radius: radius, font: k.m.font,
                   press: k.reduce ? 0 : max(key.press.value(t), bob), ink: key.ink.value(t), dark: dark)
              .scaleEffect(k.reduce ? 1 : 0.92 + 0.08 * p, anchor: .bottom)
              .blur(radius: k.reduce ? 0 : (1 - p) * 3)
              .opacity(p)
              // Keys land: they arrive from a little above and leave sinking.
              .offset(y: k.reduce ? 0 : CGFloat(1 - p) * (key.presence.to > 0.5 ? -4 : 2))
              .offset(x: w + CGFloat(key.x.value(t)) - kw / 2)
          }
        }
      }
    }
    .frame(width: w, height: k.m.height, alignment: .topLeading)
  }
}

enum Breath {
  /// 1…0…1 every `period` seconds: the listening dot and caret.
  static func at(_ t: Double, _ period: Double = 1.3) -> Double { 0.5 + 0.5 * cos(2 * .pi * t / period) }
}

// MARK: Field

/// One shortcut's field at time `t`: the keys in a recessed field that reads as something to
/// click. Listening tints it red, a dot breathes, and keys land as they go down.
struct ShortcutField: View {
  static let metrics = KeyMetrics(height: 26, unit: 26, gap: 4, pad: 4, lead: 18, slot: 8, empty: 100, font: 13)

  let k: KeyMotion
  let t: Double
  let theme: SettingsTheme
  var hovering = false

  var body: some View {
    let listen = k.listen.value(t)
    let w = CGFloat(k.width.value(t))
    let h = k.m.height + k.m.pad * 2 - 1
    let breath = k.reduce ? 1 : Breath.at(t)
    // How much of a key is on screen: the prompt gives way to the keys and the caret follows them.
    let keyed = k.keys.values.map { $0.presence.value(t) }.max() ?? 0
    let hover = theme.dark ? Color.white.opacity(0.16) : Color.black.opacity(0.17)
    ZStack(alignment: .topLeading) {
      RoundedRectangle(cornerRadius: 10, style: .continuous)
        .fill(mix(theme.dark ? Color.black.opacity(0.22) : Color.black.opacity(0.04),
                  keyInk.opacity(theme.dark ? 0.12 : 0.045), listen))
        .overlay(
          RoundedRectangle(cornerRadius: 10, style: .continuous)
            .strokeBorder(mix(hovering ? hover : theme.groupLine, keyInk.opacity(0.5), listen), lineWidth: 1)
        )
        .frame(width: w, height: h)
      Circle().fill(keyInk).frame(width: 7, height: 7)
        .scaleEffect(0.8 + 0.2 * breath)
        .opacity(listen * (0.55 + 0.45 * breath))
        .offset(x: k.m.pad + 6, y: h / 2 - 3.5)
      Text("Type a shortcut")
        .font(.system(size: 12))
        .foregroundStyle(theme.secondary)
        .fixedSize()
        // Gone by the time the first key is half in, so the two never overlap.
        .opacity(listen * max(0, 1 - 2 * keyed))
        .blur(radius: k.reduce ? 0 : (1 - listen * max(0, 1 - 2 * keyed)) * 2)
        .offset(x: k.m.pad + k.m.lead, y: h / 2 - 8)
      Text("Set shortcut")
        .font(.system(size: 12))
        .foregroundStyle(theme.secondary)
        .fixedSize()
        .frame(width: w, height: h)
        .opacity(k.blank.value(t))
        .blur(radius: k.reduce ? 0 : (1 - k.blank.value(t)) * 2)
      Capsule().fill(keyInk).frame(width: 1.5, height: 14)
        .opacity(listen * keyed * (0.35 + 0.65 * breath))
        .offset(x: w + CGFloat(k.caret.value(t)) - 0.75, y: h / 2 - 7)
      KeyRow(k: k, t: t, dark: theme.dark, ripple: true)
        .offset(y: k.m.pad - 0.5)
    }
    .frame(width: w, height: h, alignment: .topLeading)
    // Keys leaving while the field narrows fade under its edge, never cut by it.
    .mask(alignment: .topLeading) {
      RoundedRectangle(cornerRadius: 10, style: .continuous)
        .frame(width: w + 4, height: h + 4).offset(x: -2, y: -2).blur(radius: 1.5)
    }
    .offset(x: k.shake(t))
  }
}


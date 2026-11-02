import AVFoundation
import AppKit
import ApplicationServices
import Carbon.HIToolbox
import CoreGraphics
import CoreText
import Speech
import SwiftUI

// `apparent-helper onboarding`: the welcome window. Apparent decides what it shows
// (`src/onboarding.rs`) and sends it as JSON lines on stdin:
//   {"type":"state","step":"permissions","index":2,"count":4,"rows":[{"permission":"microphone",
//     "status":"not-determined","requested":false,"required":true,"reopening":false,
//     "relaunch":false},…],"ready":false,"fix":false,"tutorial":null|"skipped"|"watched"|"practiced"}
//   {"type":"request","permission":"microphone"}      ask macOS (the system prompt)
//   {"type":"open-settings","permission":"camera"}    its System Settings pane
//   {"type":"hide"} {"type":"show"} {"type":"close"}
// Buttons come back on stdout as {"action":"continue"|"back"|"later"|"close"|"settings"|"skip"} or
// {"action":"grant","permission":"microphone"} for a row's button; the how-it-works demo also
// sends {"action":"watched"} after one play-through and {"action":"practiced"} after a mark.
//
// `apparent-helper permissions` prints what Apparent has been granted, and
// `apparent-helper onboarding --render DIR` writes every step as a PNG.

// MARK: Permissions

enum Permissions {
  static func statuses() -> [String: String] {
    func av(_ type: AVMediaType) -> String {
      switch AVCaptureDevice.authorizationStatus(for: type) {
      case .authorized: return "granted"
      case .denied, .restricted: return "denied"
      default: return "not-determined"
      }
    }
    var statuses = [
      "screen-recording": CGPreflightScreenCaptureAccess() ? "granted" : "not-determined",
      "microphone": av(.audio),
      "accessibility": AXIsProcessTrusted() ? "granted" : "not-determined",
      "camera": av(.video),
    ]
    // Only SFSpeechRecognizer, the macOS 15 transcriber, needs it; SpeechAnalyzer doesn't.
    if !SpeechModel.analyzer {
      switch SFSpeechRecognizer.authorizationStatus() {
      case .authorized: statuses["speech-recognition"] = "granted"
      case .denied, .restricted: statuses["speech-recognition"] = "denied"
      default: statuses["speech-recognition"] = "not-determined"
      }
    }
    return statuses
  }

  static func request(_ permission: String) {
    switch permission {
    case "microphone": AVCaptureDevice.requestAccess(for: .audio) { _ in }
    case "camera": AVCaptureDevice.requestAccess(for: .video) { _ in }
    case "speech-recognition": SFSpeechRecognizer.requestAuthorization { _ in }
    case "accessibility":
      let prompt = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
      _ = AXIsProcessTrustedWithOptions([prompt: true] as CFDictionary)
    case "screen-recording": _ = CGRequestScreenCaptureAccess()
    default: break
    }
  }

  /// The drag panels docked beside System Settings, told when their row is granted.
  static var panels: [String: Process] = [:]

  /// The System Settings pane; for lists an app can be dragged into, the drag panel flies there
  /// from the row's button (`button`, screen coordinates).
  static func openSettings(_ permission: String, from button: NSRect? = nil) {
    if PermissionPanel.Kind(rawValue: permission) != nil {
      if let old = panels[permission] { PermissionPanel.tell(old, "cancel") }
      panels[permission] = PermissionPanel.launch(permission: permission, from: button) { _ in }
      return
    }
    let anchor = switch permission {
    case "camera": "Privacy_Camera"
    case "speech-recognition": "Privacy_SpeechRecognition"
    default: "Privacy_Microphone"
    }
    if let url = URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?\(anchor)") {
      NSWorkspace.shared.open(url)
    }
  }

  static func closeGuide(_ permission: String) {
    if let panel = panels.removeValue(forKey: permission) { PermissionPanel.tell(panel, "granted") }
  }
}

// MARK: State

struct PermissionRow: Decodable, Equatable, Identifiable {
  var permission: String
  var status = "not-determined"
  var requested = false
  var required = true
  var reopening = false
  var relaunch = false

  var id: String { permission }
  var granted: Bool { status == "granted" }
}

/// Apple's speech model, which Apparent installs in the background (`src/speech_model.rs`).
struct SpeechStatus: Decodable, Equatable {
  var state = "unknown"
  var engine: String?
  var locale: String?
  var progress = 0.0
  var locales: [String]?
  var error: String?

  private var language: String {
    locale.flatMap { Locale.current.localizedString(forIdentifier: $0) } ?? "your language"
  }

  private var percent: String { progress > 0 ? " \(Int(progress * 100))%" : "" }
  var percentInParens: String { progress > 0 ? " (\(Int(progress * 100))%)" : "" }

  /// One line for onboarding and Settings, or nil when there's nothing to say.
  var line: String? {
    switch state {
    case "checking": return "Checking for the speech model…"
    case "downloading": return "Downloading the speech model…\(percent)"
    case "ready": return "Speech model ready (\(language))"
    case "unavailable": return "No Apple speech model for \(language) on this Mac"
    // macOS 15: the error says what to turn on (Speech Recognition, or Dictation's model).
    case "needsPermission": return error ?? "Speech Recognition isn't allowed yet, so captures have no transcript."
    case "failed" where engine == "SFSpeechRecognizer" && error != nil: return error
    case "failed": return "The speech model didn't download. apparent will try again."
    default: return nil
    }
  }

  /// Still on its way, or not coming: worth a line on the Microphone row and Try it.
  var notReady: Bool { ["checking", "downloading", "failed", "unavailable", "needsPermission"].contains(state) }
}

struct OnboardingState: Decodable, Equatable {
  var step = "welcome"
  var index = 0
  var count = 4
  var rows: [PermissionRow] = []
  var ready = false
  var fix = false
  var shortcutEditing = false
  var firstCaptureRequired = false
  var shortcutChosen = false
  var pickerNote = false
  var speech: SpeechStatus?
  /// The demo on how-it-works: nil, "skipped", "watched" or "practiced".
  var tutorial: String?

  /// The demo has played through (or been drawn on): its button reads Continue, not Skip.
  var tutorialSeen: Bool { tutorial == "watched" || tutorial == "practiced" }

  /// A fix is shorter than the tour: it has no art or page dots.
  var height: CGFloat {
    guard fix else {
      // Four permission rows fit alone, but the picker note needs its own space.
      return 600 + (step == "permissions" && pickerNote && rows.count >= 4 ? 76 : 0)
    }
    return CGFloat(360 + 76 * rows.count)
  }
}

/// The Record row sent by the same Settings panel that validates and saves shortcuts.
struct OnboardingShortcut: Decodable {
  var label: String
  var recording: Bool
  var externallyManaged: Bool
  var isDefault: Bool
  var error: String?
}

let stepOrder = ["welcome", "how-it-works", "permissions", "shortcut", "try-it", "done"]

final class OnboardingModel: ObservableObject {
  @Published var state = OnboardingState()
  /// The last step change went forward, so pages slide in from the trailing edge.
  @Published var forward = true
  @Published var shortcut: String
  @Published var shortcutEditor: OnboardingShortcut?
  let paste: String
  let icon: NSImage?
  var send: (String) -> Void = { _ in }
  var grant: (String) -> Void = { _ in }
  /// Each row button's frame in the window (SwiftUI global, top-left), where the drag panel
  /// flies from.
  var buttonFrames: [String: CGRect] = [:]

  init(shortcut: String, paste: String, icon: NSImage?) {
    self.shortcut = shortcut
    self.paste = paste
    self.icon = icon
  }

  /// `⌥⇧S` → (`⌥⇧`, `S`).
  var shortcutParts: (mods: String, key: String) {
    let chars = Array(shortcut)
    let split = chars.firstIndex { !"⌃⌥⇧⌘".contains($0) } ?? chars.count
    return (String(chars[..<split]), String(chars[split...]))
  }
}

// MARK: Palette (matches the review window and HUD)

enum Palette {
  static let canvas = Color(red: 0x16 / 255, green: 0x16 / 255, blue: 0x16 / 255)
  static let surface = Color(red: 0x1D / 255, green: 0x1D / 255, blue: 0x1D / 255)
  static let raised = Color(red: 0x25 / 255, green: 0x25 / 255, blue: 0x25 / 255)
  static let line = Color.white.opacity(0.09)
  static let text = Color(red: 0xF2 / 255, green: 0xF2 / 255, blue: 0xF2 / 255)
  static let soft = Color(red: 0xD0 / 255, green: 0xD0 / 255, blue: 0xD0 / 255)
  static let muted = Color(red: 0x96 / 255, green: 0x96 / 255, blue: 0x96 / 255)
  static let ink = Color(red: 1.0, green: 0x3B / 255, blue: 0x30 / 255)
  static let positive = Color(red: 0x69 / 255, green: 0xD8 / 255, blue: 0x9F / 255)
}

// The product demonstration keeps Palette; the tour follows the system appearance.
enum TourBrand {
  private static func adaptive(light: UInt32, dark: UInt32) -> NSColor {
    NSColor(name: nil) { appearance in
      let value = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
      return NSColor(red: CGFloat((value >> 16) & 255) / 255,
                     green: CGFloat((value >> 8) & 255) / 255,
                     blue: CGFloat(value & 255) / 255, alpha: 1)
    }
  }
  static let paperNS = adaptive(light: 0xf5f0e8, dark: 0x201b24)
  static let paper = Color(nsColor: paperNS)
  static let stock = Color(nsColor: adaptive(light: 0xe7e0ee, dark: 0x302a36))
  static let carbon = Color(nsColor: adaptive(light: 0x020102, dark: 0xf5f0e8))
  static let lilac = Color(red: 183/255, green: 165/255, blue: 216/255)
  static let signal = Color(red: 122/255, green: 88/255, blue: 199/255)
  static let success = Color(nsColor: adaptive(light: 0x146b43, dark: 0x69d89f))
  static let negative = Color(nsColor: adaptive(light: 0xb42318, dark: 0xff6b60))
  static let buttonText = Color(red: 245/255, green: 240/255, blue: 232/255)
  static let line = carbon.opacity(0.18)
}

// MARK: Motion (springs by duration and bounce, no bounce; reduced motion keeps a short fade)

/// While rendering PNGs, animations hold this phase (0…1) instead of running.
var onboardingFixedPhase: Double?

let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion

enum Motion {
  static let page: Animation = reduceMotion ? .easeOut(duration: 0.18) : .spring(duration: 0.4, bounce: 0)
  static let state: Animation = reduceMotion ? .easeOut(duration: 0.18) : .spring(duration: 0.3, bounce: 0)

  static func page(forward: Bool) -> AnyTransition {
    if reduceMotion { return .opacity }
    return .asymmetric(
      insertion: .offset(x: forward ? 24 : -24).combined(with: .opacity),
      removal: .offset(x: forward ? -24 : 24).combined(with: .opacity)
    )
  }

  /// A status replaced in place: a slight scale and fade.
  static let swap: AnyTransition = reduceMotion ? .opacity : .opacity.combined(with: .scale(scale: 0.96))
}

struct Looping<Content: View>: View {
  let period: Double
  @ViewBuilder let content: (Double) -> Content

  var body: some View {
    if let phase = onboardingFixedPhase {
      content(phase)
    } else if reduceMotion {
      content(0.3)
    } else {
      TimelineView(.animation) { context in
        content(context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: period) / period)
      }
    }
  }
}

// MARK: Building blocks

struct Keycap: View {
  let label: String
  var lit = false
  var size: CGFloat = 34

  var body: some View {
    Text(label)
      .font(.system(size: size * 0.44, weight: .medium, design: .rounded))
      .foregroundStyle(lit ? Palette.ink : Palette.text)
      .frame(minWidth: size, minHeight: size)
      .padding(.horizontal, label.count > 1 ? 8 : 0)
      .background(
        RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
          .fill(LinearGradient(colors: [Color.white.opacity(0.13), Color.white.opacity(0.06)], startPoint: .top, endPoint: .bottom))
      )
      .overlay(
        RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
          .strokeBorder(lit ? Palette.ink.opacity(0.7) : Color.white.opacity(0.14), lineWidth: 1)
      )
      .shadow(color: .black.opacity(0.45), radius: 1.5, y: 2)
  }
}

struct Keycaps: View {
  let shortcut: String
  var litMods = false
  var size: CGFloat = 34

  var body: some View {
    let chars = Array(shortcut).map(String.init)
    HStack(spacing: size * 0.18) {
      ForEach(Array(chars.enumerated()), id: \.offset) { i, c in
        Keycap(label: c, lit: i != chars.count - 1 && litMods, size: size)
      }
    }
  }
}

struct PrimaryButton: View {
  let title: String
  var enabled = true
  /// A neutral fill instead of red (Skip under the demo).
  var quiet = false
  let action: () -> Void

  var body: some View {
    let prominent = enabled && !quiet
    Button(action: action) {
      ZStack {
        Text(title)
          .id(title)
          .transition(Motion.swap)
      }
        .font(.custom("HostGrotesk_700wght", size: 18))
        .foregroundStyle(prominent ? TourBrand.buttonText : TourBrand.carbon)
        .padding(.horizontal, 22)
        .frame(minWidth: 180, minHeight: 42)
        .background(prominent ? TourBrand.signal : TourBrand.stock)
        .overlay(Rectangle().strokeBorder(TourBrand.line, lineWidth: 1))
        .opacity(enabled ? 1 : 0.55)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .disabled(!enabled)
    .keyboardShortcut(.defaultAction)
  }
}

struct QuietButton: View {
  let title: String
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      Text(title)
        .font(.custom("HostGrotesk_500wght", size: 18))
        .foregroundStyle(TourBrand.carbon)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }
}

struct RowButton: View {
  let title: String
  /// The next row to do: filled in red.
  var prominent = false
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      Text(title)
        .font(.custom("HostGrotesk_700wght", size: 18))
        .foregroundStyle(prominent ? TourBrand.buttonText : TourBrand.carbon)
        .padding(.horizontal, 12)
        .frame(height: 32)
        .background(prominent ? TourBrand.signal : TourBrand.paper)
        .overlay(Rectangle().strokeBorder(TourBrand.line, lineWidth: 1))
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }
}

struct Spinner: View {
  var body: some View {
    Looping(period: 0.9) { t in
      Circle()
        .trim(from: 0, to: 0.7)
        .stroke(Palette.muted, style: StrokeStyle(lineWidth: 1.6, lineCap: .round))
        .rotationEffect(.degrees(t * 360))
    }
    .frame(width: 11, height: 11)
  }
}

// MARK: Illustrations

struct WelcomeArt: View {
  let icon: NSImage?

  var body: some View {
    Looping(period: 4) { t in
      ZStack {
        Circle().fill(Palette.ink.opacity(0.22)).frame(width: 150, height: 150).blur(radius: 46)
        if let icon {
          Image(nsImage: icon).resizable().interpolation(.high).frame(width: 128, height: 128)
        } else {
          RoundedRectangle(cornerRadius: 28, style: .continuous).fill(Palette.surface).frame(width: 112, height: 112)
        }
      }
      .offset(y: -3 * sin(t * 2 * .pi))
    }
  }
}

struct TryArt: View {
  let shortcut: String

  var body: some View {
    Looping(period: 2.2) { t in
      ZStack {
        RoundedRectangle(cornerRadius: 22, style: .continuous)
          .strokeBorder(TourBrand.signal.opacity(0.55 * (1 - t)), lineWidth: 2)
          .frame(width: 200 + 60 * t, height: 86 + 60 * t)
        Keycaps(shortcut: shortcut, litMods: true, size: 52)
      }
    }
    .frame(height: 164)
  }
}

struct DoneArt: View {
  @State private var shown = onboardingFixedPhase != nil

  var body: some View {
    ZStack {
      Circle().fill(TourBrand.success.opacity(0.14)).frame(width: 104, height: 104)
      Image(systemName: "checkmark").font(.system(size: 42, weight: .semibold)).foregroundStyle(TourBrand.success)
    }
    .scaleEffect(shown || reduceMotion ? 1 : 0.96)
    .opacity(shown ? 1 : 0)
    .frame(height: 164)
    .onAppear { withAnimation(Motion.state) { shown = true } }
  }
}

// MARK: Permission rows

struct PermissionInfo {
  let symbol: String
  let name: String
  let why: String
}

/// What System Settings calls the Accessibility list. macOS 27 renamed it.
let accessibilityName = ProcessInfo.processInfo.isOperatingSystemAtLeast(OperatingSystemVersion(majorVersion: 27, minorVersion: 0, patchVersion: 0))
  ? "Device Control and Data Access" : "Accessibility"

func permissionInfo(_ permission: String, paste: String) -> PermissionInfo {
  switch permission {
  case "screen-recording":
    return PermissionInfo(symbol: "rectangle.dashed.badge.record", name: "Screen & System Audio Recording", why: "See your screen while you record")
  case "microphone":
    return PermissionInfo(symbol: "mic", name: "Microphone", why: "Hear what you say while you record")
  case "accessibility":
    return PermissionInfo(symbol: "accessibility", name: accessibilityName, why: "Undo with ⌘Z while you record, and paste with \(paste)")
  case "camera":
    return PermissionInfo(symbol: "iphone", name: "Camera", why: "Show and record a cabled iPhone")
  case "speech-recognition":
    return PermissionInfo(symbol: "waveform", name: "Speech Recognition", why: "Write down what you say, on this Mac")
  default:
    return PermissionInfo(symbol: "lock", name: permission, why: "")
  }
}

struct PermissionRowView: View {
  let row: PermissionRow
  let paste: String
  let showOptional: Bool
  /// Replaces the row's reason once it's on (the speech model's download, for Microphone).
  var detail: String?
  /// The first row still to turn on.
  var next = false
  var buttonFrame: (CGRect) -> Void = { _ in }
  let grant: () -> Void

  var body: some View {
    let info = permissionInfo(row.permission, paste: paste)
    HStack(spacing: 12) {
      ZStack {
        Circle()
          .fill(row.granted ? TourBrand.success.opacity(0.14) : TourBrand.paper)
        Image(systemName: info.symbol)
          .font(.system(size: 14, weight: .medium))
          .foregroundStyle(row.granted ? TourBrand.success : TourBrand.carbon)
      }
      .frame(width: 32, height: 32)
      VStack(alignment: .leading, spacing: 2) {
        HStack(spacing: 6) {
          Text(info.name)
            .font(.custom("HostGrotesk_700wght", size: 18))
            .foregroundStyle(TourBrand.carbon)
          if showOptional && !row.required {
            Text("Optional")
              .font(.custom("HostGrotesk_500wght", size: 16))
              .foregroundStyle(TourBrand.carbon)
              .padding(.horizontal, 6)
              .padding(.vertical, 1.5)
              .background(TourBrand.paper)
          }
        }
        Text(row.granted ? detail ?? info.why : info.why)
          .font(.custom("HostGrotesk_400wght", size: 18))
          .foregroundStyle(TourBrand.carbon)
      }
      Spacer(minLength: 8)
      trailing
    }
    .padding(.horizontal, 14)
    .frame(height: 76)
  }

  @ViewBuilder private var trailing: some View {
    ZStack(alignment: .trailing) {
      if row.granted && row.reopening {
        HStack(spacing: 6) {
          Spinner()
          Text("Reopening apparent")
        }
        .transition(Motion.swap)
      } else if row.granted && row.relaunch {
        RowButton(title: "Reopen apparent", prominent: true, action: grant)
          .transition(Motion.swap)
      } else if row.granted {
        HStack(spacing: 5) {
          Image(systemName: "checkmark.circle.fill").foregroundStyle(TourBrand.success)
          Text("Allowed")
        }
        .transition(Motion.swap)
      } else {
        // A list Apparent can be dragged into gets the drag card either way, so it says Allow
        // until it has been pressed, even when macOS already lists Apparent as switched off.
        let draggable = PermissionPanel.Kind(rawValue: row.permission) != nil
        let settings = row.requested || (row.status == "denied" && !draggable)
        HStack(spacing: 8) {
          if row.requested && (row.status != "denied" || draggable) {
            Spinner()
          }
          RowButton(title: settings ? "Open Settings" : "Allow", prominent: next && !row.requested, action: grant)
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { buttonFrame($0) }
        }
        .transition(Motion.swap)
      }
    }
    .font(.custom("HostGrotesk_500wght", size: 18))
    .foregroundStyle(TourBrand.carbon)
  }
}

// MARK: Steps

struct OnboardingView: View {
  @ObservedObject var model: OnboardingModel

  var state: OnboardingState { model.state }

  var body: some View {
    ZStack {
      TourBrand.paper
      VStack(spacing: 0) {
        page
          .id(state.step)
          .transition(Motion.page(forward: model.forward))
        Spacer(minLength: 0)
        footer
      }
      .padding(.horizontal, 40)
      .padding(.bottom, 20)
    }
    .animation(Motion.page, value: state.step)
    .animation(Motion.state, value: state.rows)
    .animation(Motion.state, value: state.ready)
    .animation(Motion.state, value: state.tutorialSeen)
    .frame(width: 620, height: state.height)
  }

  @ViewBuilder private var page: some View {
    switch state.step {
    case "how-it-works": howItWorks
    case "permissions": permissions
    default: centered
    }
  }

  // Welcome, Try it and Done: art, a title and a line or two.
  private var centered: some View {
    let s = model.shortcut
    let (title, body): (String, String) = switch state.step {
    case "try-it": ("Make your first capture", state.firstCaptureRequired
      ? "Tap \(s) to start. Try giving feedback on this onboarding: talk and point at something. Tap \(s) again to finish. The capture stays local; nothing is sent to us."
      : "Tap \(s) now. Talk, point at something, then tap \(s) again.")
    case "done": ("It's on your clipboard", "Press ⌘V in your agent. apparent stays in the menu bar; tap \(s) whenever you want to show something.")
    case "shortcut": ("Choose your shortcut", state.firstCaptureRequired
      ? (state.shortcutChosen
          ? "Shortcut saved. Continue to make your first capture."
          : "Choose a key combination and press it here. You will use it to start and finish recordings.")
      : state.shortcutEditing
      ? (model.shortcutEditor?.recording == true
          ? "Press the new shortcut now. Esc cancels; Continue keeps the current one."
          : "Continue to try it, or change it again.")
      : "Record with \(s). You can change it now, or keep it and move on.")
    default: ("Welcome to apparent", "Record your screen and voice. Mark what matters, then paste the capture into your agent.")
    }
    return VStack(spacing: 0) {
      Spacer().frame(height: 34)
      Group {
        switch state.step {
        case "try-it": TryArt(shortcut: s)
        case "shortcut": Keycaps(shortcut: s, litMods: true, size: 52)
        case "done": DoneArt()
        default: brandWordmark
        }
      }
      .frame(height: state.step == "shortcut" && state.shortcutEditing ? 160 : 200)
      Text(title)
        .font(.custom("HostGrotesk_700wght", size: 36))
        .foregroundStyle(TourBrand.carbon)
        .multilineTextAlignment(.center)
        .padding(.top, 18)
      Text(body)
        .font(.custom("HostGrotesk_400wght", size: 18))
        .lineSpacing(3)
        .foregroundStyle(TourBrand.carbon)
        .multilineTextAlignment(.center)
        .frame(maxWidth: 440)
        .fixedSize(horizontal: false, vertical: true)
        .padding(.top, 10)
      if state.step == "shortcut" {
        if state.shortcutEditing, let editor = model.shortcutEditor {
          VStack(spacing: 8) {
            Keycaps(shortcut: editor.label, litMods: editor.recording, size: 30)
              .padding(.horizontal, 18)
              .padding(.vertical, 10)
              .background(TourBrand.stock)
              .overlay(Rectangle().strokeBorder(TourBrand.line, lineWidth: 1))
            Text(editor.externallyManaged
              ? "This shortcut belongs to your desktop. Change it in your system settings."
              : editor.recording ? "Press your new key combination" : "Current shortcut: \(editor.label)")
              .foregroundStyle(TourBrand.carbon)
            if let error = editor.error { Text(error).foregroundStyle(TourBrand.negative) }
            if editor.recording {
              QuietButton(title: "Cancel") { model.send("cancel-shortcut") }
            }
            if !state.firstCaptureRequired && !editor.isDefault && !editor.externallyManaged {
              QuietButton(title: "Reset to default") { model.send("reset-shortcut") }
            }
          }
          .padding(.top, 12)
        }
        if !state.shortcutEditing || model.shortcutEditor?.recording == false {
          QuietButton(title: state.firstCaptureRequired ? "Set shortcut" : "Change shortcut") { model.send("change-shortcut") }
            .padding(.top, 18)
        }
      }
      if state.step == "try-it" {
        if state.pickerNote {
          note(pickerNoteText).padding(.top, 12)
        }
        if state.speech?.notReady == true, let line = state.speech?.line {
          note((state.speech?.state == "downloading" || state.speech?.state == "checking") && state.speech?.engine != "SFSpeechRecognizer"
            ? "The speech model is still downloading\(state.speech?.percentInParens ?? ""). If it isn't done when you finish, apparent adds the transcript to your capture afterwards."
            : line)
            .padding(.top, 12)
        }
      }
    }
  }

  private var brandWordmark: some View {
    let url = Bundle.main.url(forResource: "apparent-wordmark", withExtension: "png")
      ?? URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appending(path: "assets/brand/apparent-wordmark.png")
    return Group {
      if let image = NSImage(contentsOf: url) {
        Image(nsImage: image).resizable().frame(width: 208, height: 48)
      }
    }
    .accessibilityLabel("apparent")
  }

  private func heading(_ title: String, _ body: String?) -> some View {
    VStack(spacing: 0) {
      Text(title)
        .font(.custom("HostGrotesk_700wght", size: 36))
        .foregroundStyle(TourBrand.carbon)
        .multilineTextAlignment(.center)
      if let body {
        Text(body)
          .font(.custom("HostGrotesk_400wght", size: 18))
          .lineSpacing(3)
          .foregroundStyle(TourBrand.carbon)
          .multilineTextAlignment(.center)
          .frame(maxWidth: 480)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.top, 8)
      }
    }
    .padding(.top, 44)
  }

  private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
    VStack(spacing: 0, content: content)
      .background(TourBrand.stock)
      .overlay(Rectangle().strokeBorder(TourBrand.line, lineWidth: 1))
  }

  private var divider: some View {
    Rectangle().fill(TourBrand.line).frame(height: 1).padding(.leading, 16)
  }

  private var howItWorks: some View {
    VStack(spacing: 0) {
      Text("How apparent works")
        .font(.custom("HostGrotesk_700wght", size: 36))
        .foregroundStyle(TourBrand.carbon)
        .padding(.top, 34)
      HowItWorksDemo(shortcut: model.shortcut, paste: model.paste, tutorial: state.tutorial, send: model.send)
        .padding(.top, 16)
    }
  }

  private var permissionsCopy: (title: String, body: String?) {
    let required = state.rows.filter(\.required).count
    if state.fix, state.rows.count == 1, state.rows.first?.permission == "camera" {
      return ("Camera, for your iPhone", "macOS treats a cabled iPhone's screen as a camera, so apparent needs Camera access to show and record it. Pick iPhone again once it's on.")
    }
    if state.fix {
      return ("apparent can't record yet", "Turn these on, then press \(model.shortcut) again.")
    }
    let count = ["zero", "one", "two", "three", "four", "five"][min(required, 5)]
    return ("apparent needs \(count) permissions", nil)
  }

  private var permissions: some View {
    let copy = permissionsCopy
    let next = state.rows.first { !$0.granted }?.permission
    return VStack(spacing: 0) {
      heading(copy.title, copy.body)
      card {
        ForEach(Array(state.rows.enumerated()), id: \.element.id) { i, row in
          if i > 0 { divider }
          PermissionRowView(
            row: row, paste: model.paste, showOptional: !state.fix,
            detail: row.permission == speechRow && state.speech?.notReady == true ? state.speech?.line : nil,
            next: row.permission == next,
            buttonFrame: { model.buttonFrames[row.permission] = $0 },
            grant: { model.grant(row.permission) }
          )
        }
      }
      .padding(.top, 22)
      if state.pickerNote {
        note(pickerNoteText)
          .padding(.top, 14)
      }
    }
  }

  /// The row that carries the speech model's line: Speech Recognition where it's asked for
  /// (macOS 15), else Microphone.
  private var speechRow: String {
    state.rows.contains { $0.permission == "speech-recognition" } ? "speech-recognition" : "microphone"
  }

  private var pickerNoteText: String {
    "macOS may also ask whether apparent can bypass its private window picker. Choose Allow, so apparent can record the screen you're showing."
  }

  private func note(_ text: String) -> some View {
    Text(text)
      .font(.custom("HostGrotesk_400wght", size: 16))
      .lineSpacing(2)
      .foregroundStyle(TourBrand.carbon)
      .multilineTextAlignment(.center)
      .frame(maxWidth: 480)
      .fixedSize(horizontal: false, vertical: true)
      .transition(.opacity)
  }

  private var primary: (title: String, action: String, enabled: Bool)? {
    switch state.step {
    case "welcome": return ("Get Started", "continue", true)
    case "how-it-works": return state.tutorialSeen ? ("Continue", "continue", true) : ("Skip", "skip", true)
    case "permissions":
      return (state.fix ? "Done" : "Continue", "continue", state.ready)
    case "shortcut": return ("Continue", "continue", !state.firstCaptureRequired || state.shortcutChosen)
    case "done": return ("Done", "continue", true)
    default: return nil
    }
  }

  private var secondary: String? {
    switch state.step {
    case "permissions": return state.fix ? "Not now" : "Set up later"
    case "try-it": return state.firstCaptureRequired ? "Try later" : "Close"
    case "done": return "Open Settings"
    default: return nil
    }
  }

  private var footer: some View {
    VStack(spacing: 10) {
      if let primary {
        PrimaryButton(title: primary.title, enabled: primary.enabled, quiet: primary.action == "skip") { model.send(primary.action) }
      }
      if let secondary {
        QuietButton(title: secondary) {
          model.send(state.step == "try-it" && !state.firstCaptureRequired ? "continue" : state.step == "done" ? "settings" : "later")
        }
      } else {
        Spacer().frame(height: 22)
      }
      ZStack {
        if state.step != "done" && !state.fix {
          HStack(spacing: 6) {
            ForEach(0..<max(state.count, 1), id: \.self) { i in
              Capsule()
                .fill(i == state.index ? TourBrand.signal : TourBrand.carbon.opacity(0.2))
                .frame(width: i == state.index ? 16 : 6, height: 6)
            }
          }
        }
        HStack {
          if state.index > 0 && !state.fix && state.step != "done" {
            QuietButton(title: "Back") { model.send("back") }
          }
          Spacer()
        }
      }
      .frame(height: 22)
      .padding(.top, 8)
    }
  }
}

// MARK: Window

/// Takes the first click while Apparent isn't the active app, so a button works on the first try.
final class FirstClickHostingView<Content: View>: NSHostingView<Content> {
  override func acceptsFirstMouse(for _: NSEvent?) -> Bool { true }
}

/// The keyboard for a helper window (onboarding, Settings). These helpers are menu bar
/// (accessory) apps with no main menu, so ⌘W would do nothing, and when one exits macOS picks
/// some app to come forward. This closes on ⌘W and gives the keyboard back to the app that was
/// in front when the window came up: Apparent if its review window opened Settings, else the app
/// the user was in.
final class HelperWindowKeys {
  private var monitor: Any?
  private var previous: NSRunningApplication?

  /// ⌘W closes the window while `enabled` (Settings turns it off while a shortcut recorder
  /// listens, so ⌘W can be recorded).
  init(window: @escaping () -> NSWindow?, enabled: @escaping () -> Bool = { true }) {
    monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
      let flags = event.modifierFlags.intersection([.command, .shift, .option, .control])
      guard flags == .command, event.charactersIgnoringModifiers?.lowercased() == "w", enabled(),
            let window = window()
      else { return event }
      window.performClose(nil)
      return nil
    }
  }

  /// Brings the window forward with the keys. The cooperative `NSApp.activate()` is refused
  /// when Apparent isn't the active app (opened from its status menu, say), which left the window
  /// behind the app in front, taking that app's keys; gpui's `cx.activate(true)` uses the same
  /// call as here. The app in front is remembered on every show, so a Settings window shown
  /// again from the review window returns there.
  func bringForward(_ window: NSWindow?) {
    if let front = NSWorkspace.shared.frontmostApplication,
       front.processIdentifier != ProcessInfo.processInfo.processIdentifier
    {
      previous = front
    }
    window?.orderFrontRegardless()
    window?.makeKey()
    NSApp.activate(ignoringOtherApps: true)
  }

  /// Before the helper exits.
  func handBack() {
    guard NSApp.isActive, let previous, !previous.isTerminated else { return }
    previous.activate(options: [])
  }
}

final class Onboarding: NSObject, NSApplicationDelegate, NSWindowDelegate {
  static func run(arguments: [String]) -> Never {
    let font = Bundle.main.url(forResource: "HostGrotesk-Variable", withExtension: "ttf")
      ?? URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appending(path: "assets/fonts/HostGrotesk-Variable.ttf")
    CTFontManagerRegisterFontsForURL(font as CFURL, .process, nil)
    let model = OnboardingModel(
      shortcut: option("--shortcut", in: arguments) ?? "⌥⇧S",
      paste: option("--paste", in: arguments) ?? "⌥⇧V",
      icon: loadIcon(option("--icon", in: arguments))
    )
    if let dir = option("--render", in: arguments) {
      MainActor.assumeIsolated {
        // Preview both appearances without changing the tester's system preference.
        let scheme: ColorScheme = arguments.contains("--dark") ? .dark : .light
        NSApplication.shared.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        if let fps = option("--demo-frames", in: arguments).flatMap(Double.init) {
          renderDemo(model: model, fps: fps, to: URL(filePath: dir), scheme: scheme)
        } else {
          render(model: model, to: URL(filePath: dir), scheme: scheme)
        }
      }
      exit(0)
    }
    let app = NSApplication.shared
    let delegate = Onboarding(model: model)
    app.setActivationPolicy(.accessory)
    app.delegate = delegate
    withExtendedLifetime(delegate) { app.run() }
    exit(0)
  }

  static func loadIcon(_ path: String?) -> NSImage? {
    if let path, let image = NSImage(contentsOfFile: path) { return image }
    // Inside Apparent.app: Contents/MacOS/apparent-helper.
    let bundle = URL(filePath: CommandLine.arguments[0]).standardizedFileURL
      .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    return bundle.pathExtension == "app" ? NSWorkspace.shared.icon(forFile: bundle.path) : nil
  }

  let model: OnboardingModel
  var window: NSWindow?
  private var shortcutMonitor: Any?
  private lazy var keys = HelperWindowKeys(
    window: { [weak self] in self?.window },
    enabled: { [weak self] in self?.model.shortcutEditor?.recording != true }
  )

  init(model: OnboardingModel) {
    self.model = model
  }

  func applicationDidFinishLaunching(_: Notification) {
    if let index = CommandLine.arguments.firstIndex(of: "--validate-shortcut") {
      let args = CommandLine.arguments
      guard args.count == index + 3,
            let key = Int(args[index + 1]), (0...127).contains(key),
            let modifiers = Int(args[index + 2]), modifiers >= 0,
            modifiers & ~0x1b00 == 0 else { exit(1) }
      print(ShortcutKeys.reserved(keyCode: key, modifiers: modifiers) ? "true" : "false")
      exit(0)
    }
    if CommandLine.arguments.contains("--permissions-only") {
      readCommands()
      return
    }
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 620, height: 500),
      styleMask: [.titled, .closable, .fullSizeContentView],
      backing: .buffered,
      defer: false
    )
    window.titlebarAppearsTransparent = true
    window.titleVisibility = .hidden
    window.title = "Welcome to apparent"
    window.isMovableByWindowBackground = true
    // The native first frame and SwiftUI canvas resolve against the same live appearance.
    window.backgroundColor = TourBrand.paperNS
    window.isReleasedWhenClosed = false
    // An ordinary window: System Settings and macOS's prompts always come up above it.
    window.level = .normal
    window.delegate = self
    window.contentView = FirstClickHostingView(rootView: OnboardingView(model: model))
    window.center()
    self.window = window
    _ = keys
    model.send = { action in emit(["action": action]) }
    model.grant = { permission in emit(["action": "grant", "permission": permission]) }
    readCommands()
  }

  func windowWillClose(_: Notification) {
    listenForShortcut(false)
    emit(["action": "close"])
    keys.handBack()
    exit(0)
  }

  private func present() {
    keys.bringForward(window)
  }

  func windowDidResignKey(_: Notification) {
    if model.shortcutEditor?.recording == true { emit(["action": "cancel-shortcut"]) }
  }

  private func listenForShortcut(_ on: Bool) {
    if on, shortcutMonitor == nil {
      shortcutMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
        guard let self, self.model.shortcutEditor?.recording == true else { return event }
        if event.keyCode == UInt16(kVK_Escape) {
          emit(["action": "cancel-shortcut"])
          return nil
        }
        let flags = event.modifierFlags.intersection([.command, .shift, .option, .control])
        let typed = event.characters(byApplyingModifiers: []) ?? ""
        let modifiers = ShortcutKeys.carbon(flags)
        emit(["action": "shortcut", "kind": "record", "keyCode": Int(event.keyCode),
              "modifiers": modifiers, "key": typed,
              "reserved": ShortcutKeys.reserved(keyCode: Int(event.keyCode), modifiers: modifiers)])
        return nil
      }
    } else if !on, let shortcutMonitor {
      NSEvent.removeMonitor(shortcutMonitor)
      self.shortcutMonitor = nil
    }
  }

  private func apply(_ state: OnboardingState) {
    for row in state.rows where row.granted {
      Permissions.closeGuide(row.permission)
    }
    // A grant usually lands in System Settings (or a macOS prompt): come back in front.
    let wasOff = Set(model.state.rows.filter { !$0.granted }.map(\.permission))
    if state.rows.contains(where: { $0.granted && wasOff.contains($0.permission) }) {
      returnAfterGrant()
    }
    if let window, abs((window.contentView?.frame.height ?? 0) - state.height) > 0.5 {
      window.setContentSize(NSSize(width: 620, height: state.height))
    }
    let old = stepOrder.firstIndex(of: model.state.step) ?? 0
    let new = stepOrder.firstIndex(of: state.step) ?? 0
    guard old != new else {
      model.state = state
      return
    }
    // The page transition reads the direction, so it changes a frame before the step.
    model.forward = new >= old
    DispatchQueue.main.async { self.model.state = state }
  }

  private func readCommands() {
    Thread.detachNewThread {
      while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = object["type"] as? String
        else { continue }
        let permission = object["permission"] as? String ?? ""
        let state = type == "state" ? try? JSONDecoder().decode(OnboardingState.self, from: data) : nil
        DispatchQueue.main.async {
          if CommandLine.arguments.contains("--permissions-only"),
             !["request", "open-settings", "close-guide", "close"].contains(type) { return }
          if ["request", "open-settings", "close-guide"].contains(type),
             !["screen-recording", "microphone", "camera", "accessibility", "speech-recognition"].contains(permission) { return }
          switch type {
          case "state":
            guard let state else { return }
            if state.step != "shortcut" || !state.shortcutEditing {
              self.listenForShortcut(false)
              self.model.shortcutEditor = nil
            }
            if !self.presented {
              self.model.state = state
              self.window?.setContentSize(NSSize(width: 620, height: state.height))
              self.window?.center()
              self.presented = true
              self.present()
            } else {
              self.apply(state)
            }
          // The user has moved on to another row: answering an earlier question mustn't pull
          // this window over the System Settings pane they're working in.
          case "close-guide":
            Permissions.closeGuide(permission)
          case "request":
            self.followUp?.invalidate()
            Permissions.request(permission)
          case "open-settings":
            self.followUp?.invalidate()
            Permissions.openSettings(permission, from: self.screenFrame(of: permission))
          case "shortcut":
            if let label = object["label"] as? String { self.model.shortcut = label }
          case "shortcut-editor":
            if let shortcut = object["shortcut"],
               let data = try? JSONSerialization.data(withJSONObject: shortcut),
               let editor = try? JSONDecoder().decode(OnboardingShortcut.self, from: data) {
              self.model.shortcutEditor = editor
              // Focus may have gone elsewhere before the recorder's first acknowledgement.
              // Cancel it even if windowDidResignKey saw the previous, non-recording state.
              let active = self.window?.isKeyWindow == true
              if editor.recording && !active { emit(["action": "cancel-shortcut"]) }
              self.listenForShortcut(editor.recording && active)
            }
          case "hide": self.window?.orderOut(nil)
          case "show": self.present()
          case "close":
            for panel in Permissions.panels.values { PermissionPanel.tell(panel, "cancel") }
            Permissions.panels.removeAll()
            self.keys.handBack()
            exit(0)
          default: break
          }
        }
      }
      DispatchQueue.main.async {
        for panel in Permissions.panels.values { PermissionPanel.tell(panel, "cancel") }
        Permissions.panels.removeAll()
        exit(0)
      }
    }
  }

  private var presented = false
  private var followUp: Timer?

  /// Comes forward once macOS has stopped asking. A Screen Recording grant brings System
  /// Settings' own "Quit & Reopen" alert and ScreenCaptureKit's window picker question, and
  /// answering one hands the keys back to System Settings, so for five minutes after a grant the
  /// window comes forward again each time the last question on screen is answered, until the
  /// user presses another row (never otherwise, so it doesn't fight the user going elsewhere).
  private func returnAfterGrant() {
    followUp?.invalidate()
    let started = Date()
    var asking = Self.macOSIsAsking()
    var returned = false
    if !asking, window?.isVisible == true {
      present()
      returned = true
    }
    followUp = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: true) { [weak self] timer in
      guard let self, Date().timeIntervalSince(started) < 300, self.window?.isVisible == true else {
        timer.invalidate()
        return
      }
      if Self.macOSIsAsking() {
        asking = true
      } else if asking || !returned {
        asking = false
        returned = true
        if !(NSApp.isActive && self.window?.isKeyWindow == true) { self.present() }
      }
    }
  }

  /// A macOS question is in view: a privacy alert or password sheet, or an alert in System
  /// Settings (a second window of its own) while System Settings is in front
  /// (`windows::macos_is_asking` in Apparent).
  static func macOSIsAsking() -> Bool {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    var settingsWindows = 0
    var frontOwner: String?
    for info in list {
      let owner = info[kCGWindowOwnerName as String] as? String ?? ""
      let layer = info[kCGWindowLayer as String] as? Int ?? 0
      if owner == "UserNotificationCenter" || owner == "SecurityAgent" { return true }
      guard layer == 0 else { continue }
      if frontOwner == nil { frontOwner = owner }
      if owner == "System Settings" { settingsWindows += 1 }
    }
    return frontOwner == "System Settings" && settingsWindows > 1
  }

  /// A row button's frame on screen (AppKit coordinates).
  private func screenFrame(of permission: String) -> NSRect? {
    guard let frame = model.buttonFrames[permission], let window, let content = window.contentView else { return nil }
    let local = NSRect(x: frame.minX, y: content.bounds.height - frame.maxY, width: frame.width, height: frame.height)
    return window.convertToScreen(local)
  }

  /// One loop of the how-it-works demo as `demo-NNNN.png` at `fps`, for a video of it.
  @MainActor static func renderDemo(model: OnboardingModel, fps: Double, to dir: URL, scheme: ColorScheme) {
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    let frames = Int(DemoScript.loop * fps)
    for i in 0..<frames {
      let t = Double(i) / fps
      onboardingFixedPhase = t / DemoScript.loop
      model.state = OnboardingState(step: "how-it-works", index: 1, tutorial: t >= DemoScript.end ? "watched" : nil)
      let renderer = ImageRenderer(content: OnboardingView(model: model).environment(\.colorScheme, scheme))
      renderer.scale = 2
      guard let image = renderer.cgImage else { continue }
      let url = dir.appending(path: String(format: "demo-%04d.png", i))
      try? NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])?.write(to: url)
    }
  }

  /// Every step as `NN-step.png`, for review without running the app.
  @MainActor static func render(model: OnboardingModel, to dir: URL, scheme: ColorScheme) {
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    func row(_ permission: String, _ status: String = "not-determined", requested: Bool = false, reopening: Bool = false, relaunch: Bool = false) -> PermissionRow {
      PermissionRow(permission: permission, status: status, requested: requested, required: ["screen-recording", "microphone", "accessibility"].contains(permission), reopening: reopening, relaunch: relaunch)
    }
    let none = [row("screen-recording"), row("microphone"), row("accessibility")]
    let waiting = [row("screen-recording", requested: true), row("microphone"), row("accessibility")]
    let reopening = [row("screen-recording", "granted", reopening: true), row("microphone"), row("accessibility")]
    let resumed = [row("screen-recording", "granted"), row("microphone"), row("accessibility")]
    let some = [row("screen-recording", "granted"), row("microphone", "granted"), row("accessibility", requested: true)]
    let denied = [row("screen-recording", "granted"), row("microphone", "denied"), row("accessibility", "granted")]
    let all = [row("screen-recording", "granted"), row("microphone", "granted"), row("accessibility", "granted")]
    let fallback = [row("screen-recording", "granted", relaunch: true), row("microphone", "granted"), row("accessibility", "granted")]
    let lost = [row("screen-recording", "granted"), row("microphone", "granted"), row("accessibility")]
    let sequoia = none + [row("speech-recognition")]
    let sequoiaNoDictation = all + [row("speech-recognition", "granted")]
    let noDictation = SpeechStatus(state: "failed", engine: "SFSpeechRecognizer", locale: "en_US", error: "Turn on Dictation in System Settings › Keyboard for transcripts.")
    let samples: [(String, OnboardingState, Double)] = [
      ("welcome", OnboardingState(step: "welcome", index: 0), 0.25),
      ("how-it-works", OnboardingState(step: "how-it-works", index: 1), 0),
      ("how-it-works-recording", OnboardingState(step: "how-it-works", index: 1), 1.6 / DemoScript.loop),
      ("how-it-works-mark", OnboardingState(step: "how-it-works", index: 1), 3.4 / DemoScript.loop),
      ("how-it-works-click", OnboardingState(step: "how-it-works", index: 1), 4.95 / DemoScript.loop),
      ("how-it-works-finish", OnboardingState(step: "how-it-works", index: 1), 6.4 / DemoScript.loop),
      ("how-it-works-paste", OnboardingState(step: "how-it-works", index: 1), 7.35 / DemoScript.loop),
      ("how-it-works-fixed", OnboardingState(step: "how-it-works", index: 1, tutorial: "watched"), 9.6 / DemoScript.loop),
      ("permissions-none", OnboardingState(step: "permissions", index: 2, rows: none), 0),
      ("permissions-screen-waiting", OnboardingState(step: "permissions", index: 2, rows: waiting), 0.3),
      ("permissions-screen-reopening", OnboardingState(step: "permissions", index: 2, rows: reopening), 0.3),
      ("permissions-resumed", OnboardingState(step: "permissions", index: 2, rows: resumed), 0),
      ("permissions-some", OnboardingState(step: "permissions", index: 2, rows: some), 0.3),
      ("permissions-denied", OnboardingState(step: "permissions", index: 2, rows: denied), 0),
      ("permissions-all", OnboardingState(step: "permissions", index: 2, rows: all, ready: true), 0),
      ("permissions-reopen-fallback", OnboardingState(step: "permissions", index: 2, rows: fallback), 0),
      ("permissions-picker-note", OnboardingState(step: "permissions", index: 2, rows: some, pickerNote: true, speech: SpeechStatus(state: "downloading", locale: "en_US", progress: 0.42)), 0.3),
      ("permissions-macos15", OnboardingState(step: "permissions", index: 2, rows: sequoia, speech: SpeechStatus(state: "needsPermission", engine: "SFSpeechRecognizer", locale: "en_US")), 0),
      ("permissions-macos15-picker-note", OnboardingState(step: "permissions", index: 2, rows: sequoia, pickerNote: true, speech: SpeechStatus(state: "needsPermission", engine: "SFSpeechRecognizer", locale: "en_US")), 0),
      ("permissions-macos15-no-dictation", OnboardingState(step: "permissions", index: 2, rows: sequoiaNoDictation, ready: true, speech: noDictation), 0),
      ("shortcut", OnboardingState(step: "shortcut", index: 3, count: 5), 0),
      ("shortcut-required", OnboardingState(step: "shortcut", index: 3, count: 5, firstCaptureRequired: true), 0),
      ("shortcut-chosen", OnboardingState(step: "shortcut", index: 3, count: 5, firstCaptureRequired: true, shortcutChosen: true), 0),
      ("try-it-required", OnboardingState(step: "try-it", index: 4, count: 5, firstCaptureRequired: true, shortcutChosen: true), 0.3),
      ("try-it", OnboardingState(step: "try-it", index: 4, count: 5), 0.3),
      ("try-it-downloading", OnboardingState(step: "try-it", index: 4, count: 5, pickerNote: true, speech: SpeechStatus(state: "downloading", locale: "en_US", progress: 0.42)), 0.3),
      ("done", OnboardingState(step: "done", index: 5, count: 5), 0),
      ("fix-required", OnboardingState(step: "permissions", index: 0, count: 1, rows: lost, fix: true), 0),
      ("fix-screen-reopening", OnboardingState(step: "permissions", index: 0, count: 1, rows: reopening, fix: true), 0.3),
      ("fix-camera", OnboardingState(step: "permissions", index: 0, count: 1, rows: [row("camera", requested: true)], fix: true), 0.3),
    ]
    let practice: [CGPoint] = (0...40).map { i in
      let a = Double(i) / 40 * 2.1 * Double.pi + 3.4
      return CGPoint(x: 292 + 44 * cos(a), y: 189 + 25 * sin(a))
    }
    var extras: [(String, OnboardingState, Double, () -> Void)] = [
      ("how-it-works-trying", OnboardingState(step: "how-it-works", index: 1), 2.0 / DemoScript.loop, {
        onboardingDemoPractice = ([Array(practice.prefix(26))], true, false)
      }),
      ("how-it-works-tried", OnboardingState(step: "how-it-works", index: 1, tutorial: "practiced"), 2.0 / DemoScript.loop, {
        onboardingDemoPractice = ([practice], false, true)
      }),
      ("how-it-works-reduced-motion", OnboardingState(step: "how-it-works", index: 1, tutorial: "watched"), 0, {
        onboardingDemoPractice = nil
        onboardingDemoStoryboard = true
      }),
    ]
    extras.append(("how-it-works-skipped-earlier", OnboardingState(step: "how-it-works", index: 1, tutorial: "skipped"), 0.5 / DemoScript.loop, {
      onboardingDemoStoryboard = false
    }))
    let howCount = samples.firstIndex { $0.0 == "permissions-none" } ?? 1
    var pages: [(String, OnboardingState, Double, () -> Void)] = samples.map { ($0.0, $0.1, $0.2, {}) }
    pages.insert(contentsOf: extras, at: howCount)
    for (i, (name, state, phase, setup)) in pages.enumerated() {
      setup()
      onboardingFixedPhase = phase
      model.state = state
      let renderer = ImageRenderer(content: OnboardingView(model: model).environment(\.colorScheme, scheme))
      renderer.scale = 2
      guard let image = renderer.cgImage else { continue }
      let rep = NSBitmapImageRep(cgImage: image)
      let url = dir.appending(path: String(format: "%02d-%@.png", i + 1, name))
      try? rep.representation(using: .png, properties: [:])?.write(to: url)
    }
  }
}

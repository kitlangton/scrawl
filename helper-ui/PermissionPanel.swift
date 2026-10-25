// `scrawl-helper permission-panel`: the "drag Scrawl into the list" card for the Privacy panes
// that take a dragged app (Screen Recording, Accessibility).
//
//   scrawl-helper permission-panel --permission screen-recording|accessibility
//                                  [--from X,Y,W,H] [--app PATH] [--preview] [--reduce-motion]
//   scrawl-helper permission-panel --check PERMISSION        {"granted":true|false} from a fresh process
//   scrawl-helper permission-panel --plan --settings X,Y,W,H --visible X,Y,W,H   where the card docks
//
// It opens the pane in System Settings, waits for its window to settle, flies a card from
// `--from` (the Grant button, global top-left points like CGWindow bounds) and docks it below
// the Settings window, or inside its empty lower area when there's no room (`dock`), with its
// app row lined up under the Privacy list's column. The row
// drags the real Scrawl.app, the bundle that holds the grant. stdin takes `cancel` (fade out;
// EOF on a pipe does the same), `back` (fly back to `--from`, as the card's back button does) and
// `granted` (the parent saw the grant first: show it, then fade). Events go to stdout as JSON lines and the last one is either
//   {"event":"granted","permission":P}    or    {"event":"cancelled","permission":P,"reason":R}
// with R one of back, settings-closed, no-settings-window, parent, timeout. Before that it may
// print {"event":"docked"} and {"event":"dropped"}. A drop is only a hint: the grant is read from
// a fresh `--check` process (CGPreflightScreenCaptureAccess caches its answer per process).
//
// The flight, the Settings window tracking and the drop race are adapted from AskForPermission
// (https://github.com/riko2chen/AskForPermission, MIT, Copyright (c) 2026 Riko), and the
// macOS 27 pane names from PermissionFlow (https://github.com/jaywcjlove/PermissionFlow, MIT,
// Copyright (c) 2026 小弟调调). See THIRD_PARTY in the README.

import AppKit
import ApplicationServices
import CoreGraphics
import SwiftUI

enum PermissionPanel {
  // MARK: Tuning

  /// The flight from the Grant button to the dock. Duration and bounce, as everywhere in Scrawl:
  /// critically damped, so the card leaves fast and decelerates into place without overshooting.
  /// A spring front-loads its travel, so a cross-screen hop needs a longer duration than a drawer.
  static let flightSpring = Spring(duration: 0.85, bounce: 0)
  /// Back to the button on cancel: exits run shorter than entrances.
  static let returnSpring = Spring(duration: 0.7, bounce: 0)
  /// Width leads height (and the path) by running on a shorter spring.
  static let widthLead = 0.78
  /// Peak height of the arc above the higher end, and its cap as a share of the distance.
  static let arcHeight: CGFloat = 160
  static let arcShare: CGFloat = 0.35
  /// Reduce Motion and the in-place exits: a fade with no travel.
  static let fadeDuration = 0.18

  // System Settings (macOS 26/27): a fixed 232 pt sidebar; the Privacy list starts 20 pt past it
  // and ends 20 pt before the window's right edge. The window corners are ~26 pt round.
  static let settingsListLeading: CGFloat = 252
  static let settingsListTrailing: CGFloat = 20
  static let settingsBundleID = "com.apple.systempreferences"

  // MARK: Entry

  static func run(arguments: [String]) -> Never {
    if arguments.contains("--plan") { printPlan(arguments: arguments) }
    if let raw = option("--check", in: arguments) {
      guard let kind = Kind(rawValue: raw) else { fail("permission-panel --check needs a permission") }
      emit(["granted": kind.isGrantedInThisProcess])
      exit(0)
    }
    guard let raw = option("--permission", in: arguments), let kind = Kind(rawValue: raw) else {
      fail("permission-panel needs --permission screen-recording|accessibility")
    }
    let source = option("--from", in: arguments).flatMap(parseRect)
    let app = NSApplication.shared
    let controller = Controller(
      kind: kind,
      sourceQuartz: source,
      appURL: appBundleURL(override: option("--app", in: arguments)),
      preview: arguments.contains("--preview"),
      reduceMotion: arguments.contains("--reduce-motion")
        || NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    )
    app.setActivationPolicy(.accessory)
    app.delegate = controller
    withExtendedLifetime(controller) { app.run() }
    exit(0)
  }

  static func parseRect(_ text: String) -> CGRect? {
    let parts = text.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
    guard parts.count == 4, parts[2] > 0, parts[3] > 0 else { return nil }
    return CGRect(x: parts[0], y: parts[1], width: parts[2], height: parts[3])
  }

  /// The .app that contains this helper, since that bundle is what TCC grants. Outside an app
  /// (a debug build) it falls back to the installed Scrawl.
  static func appBundleURL(override: String?) -> URL {
    if let override, !override.isEmpty { return URL(filePath: override).standardizedFileURL }
    let executable = (Bundle.main.executableURL ?? URL(filePath: CommandLine.arguments[0]))
      .resolvingSymlinksInPath()
    var url = executable.deletingLastPathComponent()
    while url.path != "/" {
      if url.pathExtension.lowercased() == "app" { return url }
      url = url.deletingLastPathComponent()
    }
    let installed = URL(filePath: NSHomeDirectory()).appending(path: "Applications/Scrawl.app")
    if FileManager.default.fileExists(atPath: installed.path) { return installed }
    return URL(filePath: "/Applications/Scrawl.app")
  }

  /// For the onboarding window: launch the panel from a button's frame (AppKit screen
  /// coordinates) and get its events on the main queue. The returned process can be told to
  /// `cancel` by writing a line to its stdin.
  @discardableResult
  static func launch(
    permission: String,
    from sourceInScreen: NSRect?,
    onEvent: @escaping ([String: Any]) -> Void
  ) -> Process? {
    let process = Process()
    process.executableURL = Bundle.main.executableURL ?? URL(filePath: CommandLine.arguments[0])
    var arguments = ["permission-panel", "--permission", permission]
    if let sourceInScreen {
      let q = quartz(fromAppKit: sourceInScreen)
      arguments += ["--from", "\(Int(q.minX)),\(Int(q.minY)),\(Int(q.width)),\(Int(q.height))"]
    }
    process.arguments = arguments
    let output = Pipe()
    process.standardOutput = output
    process.standardInput = Pipe()
    var buffer = Data()
    output.fileHandleForReading.readabilityHandler = { handle in
      let chunk = handle.availableData
      if chunk.isEmpty { handle.readabilityHandler = nil; return }
      buffer.append(chunk)
      while let newline = buffer.firstIndex(of: 0x0A) {
        let line = buffer[..<newline]
        buffer.removeSubrange(...newline)
        if let event = try? JSONSerialization.jsonObject(with: line) as? [String: Any] {
          DispatchQueue.main.async { onEvent(event) }
        }
      }
    }
    do { try process.run() } catch { return nil }
    return process
  }

  /// Sends `cancel`, `back` or `granted` to a panel started with `launch`.
  static func tell(_ process: Process, _ command: String) {
    guard process.isRunning, let input = process.standardInput as? Pipe else { return }
    try? input.fileHandleForWriting.write(contentsOf: Data((command + "\n").utf8))
  }

  // MARK: Permissions

  enum Kind: String {
    case screenRecording = "screen-recording"
    case accessibility = "accessibility"

    var anchor: String {
      switch self {
      case .screenRecording: return "Privacy_ScreenCapture"
      case .accessibility: return "Privacy_Accessibility"
      }
    }

    /// The name the pane's list is known by. macOS 27 renamed Accessibility.
    var title: String {
      switch self {
      case .screenRecording: return "Screen Recording"
      case .accessibility:
        return ProcessInfo.processInfo.isOperatingSystemAtLeast(OperatingSystemVersion(majorVersion: 27, minorVersion: 0, patchVersion: 0))
          ? "Device Control and Data Access" : "Accessibility"
      }
    }

    var settingsURL: URL {
      URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?\(anchor)")!
    }

    /// Only meaningful in a fresh process: CGPreflightScreenCaptureAccess keeps its first answer.
    var isGrantedInThisProcess: Bool {
      switch self {
      case .screenRecording: return CGPreflightScreenCaptureAccess()
      case .accessibility: return AXIsProcessTrusted()
      }
    }

    /// Runs `permission-panel --check` in a new process, off the main thread.
    func checkFresh(_ done: @escaping (Bool) -> Void) {
      DispatchQueue.global(qos: .userInitiated).async {
        let process = Process()
        process.executableURL = Bundle.main.executableURL ?? URL(filePath: CommandLine.arguments[0])
        process.arguments = ["permission-panel", "--check", rawValue]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        var granted = false
        if (try? process.run()) != nil {
          let data = pipe.fileHandleForReading.readDataToEndOfFile()
          process.waitUntilExit()
          granted = ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])?["granted"] as? Bool ?? false
        }
        DispatchQueue.main.async { done(granted) }
      }
    }
  }

  // MARK: Spring

  /// The step response of a spring from 0 to 1 at rest, by duration and bounce:
  /// ω = 2π / duration, ζ = 1 − bounce.
  struct Spring {
    let duration: Double
    let bounce: Double

    func value(_ t: Double) -> Double {
      guard t > 0 else { return 0 }
      let omega = 2 * Double.pi / duration
      let zeta = 1 - bounce
      if zeta >= 1 { return 1 - (1 + omega * t) * exp(-omega * t) }
      let damped = omega * (1 - zeta * zeta).squareRoot()
      return 1 - exp(-zeta * omega * t) * (cos(damped * t) + zeta * omega / damped * sin(damped * t))
    }

    /// Long enough that what's left is under a hundredth of a point on a screen-wide move.
    var settleTime: Double { duration * 1.8 }
  }

  // MARK: Geometry

  static var primaryHeight: CGFloat { NSScreen.screens.first?.frame.height ?? 0 }

  static func appKit(fromQuartz r: CGRect) -> NSRect {
    NSRect(x: r.minX, y: primaryHeight - r.minY - r.height, width: r.width, height: r.height)
  }

  static func quartz(fromAppKit r: NSRect) -> CGRect {
    CGRect(x: r.minX, y: primaryHeight - r.minY - r.height, width: r.width, height: r.height)
  }

  enum Dock: String { case below, inside, right, left, clamped }

  /// Where the card docks for a Settings window on a screen, both in AppKit coordinates
  /// (`visible` is the screen's `visibleFrame`, without the menu bar and Dock). In order:
  /// - below the window, its app row under the Privacy list's column, if all of it fits;
  /// - inside the window's empty lower area (the lowest 30% of these panes holds no rows or
  ///   toggles), on the list's column and 20 pt above its bottom edge, as Codex does;
  /// - beside it, right then left;
  /// - else the below position pushed onto the screen.
  static func dock(settings: NSRect, visible: NSRect, layout: Layout) -> (frame: NSRect, dock: Dock) {
    let size = layout.cardSize
    let fits = { (r: NSRect) in
      r.minX >= visible.minX && r.maxX <= visible.maxX && r.minY >= visible.minY && r.maxY <= visible.maxY
    }
    let clampX = { (x: CGFloat) in min(max(x, visible.minX + 8), visible.maxX - size.width - 8) }
    let clampY = { (y: CGFloat) in min(max(y, visible.minY + 8), visible.maxY - size.height - 8) }
    // The list's column; nudged left if the card would pass the window's right edge.
    let column = min(settings.minX + settingsListLeading - layout.gutter, settings.maxX - layout.inset - size.width)
    let gap = layout.dockGap

    let below = NSRect(x: clampX(column), y: settings.minY - gap - size.height, width: size.width, height: size.height)
    if fits(below) { return (below, .below) }

    let insideY = max(settings.minY + layout.insideBottom, visible.minY + 8)
    let inside = NSRect(x: column, y: insideY, width: size.width, height: size.height)
    if fits(inside), inside.minX >= settings.minX, inside.maxY <= settings.minY + settings.height * 0.3 {
      return (inside, .inside)
    }

    let right = NSRect(x: settings.maxX + gap, y: clampY(settings.minY), width: size.width, height: size.height)
    if fits(right) { return (right, .right) }
    let left = NSRect(x: settings.minX - gap - size.width, y: clampY(settings.minY), width: size.width, height: size.height)
    if fits(left) { return (left, .left) }

    return (NSRect(x: clampX(below.minX), y: clampY(below.minY), width: size.width, height: size.height), .clamped)
  }

  /// `dock` for a window given in CGWindow (top-left) coordinates, on the screen holding it.
  static func dockFrame(settingsQuartz: CGRect, layout: Layout) -> NSRect {
    let settings = appKit(fromQuartz: settingsQuartz)
    let screen = NSScreen.screens.first { $0.frame.contains(NSPoint(x: settings.midX, y: settings.midY)) }
      ?? NSScreen.screens.first { $0.frame.intersects(settings) } ?? NSScreen.main
    return dock(settings: settings, visible: screen?.visibleFrame ?? settings, layout: layout).frame
  }

  /// `--plan --settings X,Y,W,H --visible X,Y,W,H` (AppKit coordinates): the dock as JSON, for
  /// checking placement without a screen.
  static func printPlan(arguments: [String]) -> Never {
    guard let settings = option("--settings", in: arguments).flatMap(parseRect),
          let visible = option("--visible", in: arguments).flatMap(parseRect)
    else { fail("permission-panel --plan needs --settings X,Y,W,H --visible X,Y,W,H") }
    let plan = dock(settings: settings, visible: visible, layout: Layout(settingsWidth: settings.width))
    let f = plan.frame
    emit(["dock": plan.dock.rawValue, "frame": [f.minX, f.minY, f.width, f.height].map { Double($0) }])
    exit(0)
  }

  /// Card metrics. The app row matches the Privacy list's column in the Settings window.
  struct Layout: Equatable {
    var rowWidth: CGFloat
    let instructionHeight: CGFloat = 28
    /// Less above the instruction than below the row: line height already pads the text.
    let top: CGFloat = 7
    let padding: CGFloat = 12
    let gutter: CGFloat = 52
    let rowHeight: CGFloat = 44
    let gap: CGFloat = 4
    let inset: CGFloat = 8
    let dockGap: CGFloat = 10
    /// Inside the window: this far above its bottom edge.
    let insideBottom: CGFloat = 20
    let radius: CGFloat = 18
    let rowRadius: CGFloat = 9
    let icon: CGFloat = 26
    let iconLeading: CGFloat = 10

    init(settingsWidth: CGFloat) {
      let column = settingsWidth - PermissionPanel.settingsListLeading - PermissionPanel.settingsListTrailing
      rowWidth = max(320, min(column, 640))
    }

    var textLeading: CGFloat { iconLeading + icon + 10 }
    var rowY: CGFloat { top + instructionHeight + gap }
    var cardSize: NSSize {
      NSSize(width: gutter + rowWidth + padding, height: rowY + rowHeight + padding)
    }
  }
}

// MARK: - Card

extension PermissionPanel {
  final class CardModel: ObservableObject {
    enum Phase { case instruction, waiting, granted }
    @Published var phase = Phase.instruction
    @Published var hovering = false
    @Published var dragging = false
    @Published var contentOpacity: Double = 1
    @Published var contentScale: Double = 1
    @Published var contentBlur: Double = 0
    var onBack: () -> Void = {}
  }

  struct CardContent: View {
    @ObservedObject var model: CardModel
    let kind: Kind
    let layout: Layout
    let appName: String
    let icon: NSImage
    let interactive: Bool
    let dragView: () -> NSView

    var body: some View {
      ZStack(alignment: .topLeading) {
        instruction
          .frame(width: layout.rowWidth, height: layout.instructionHeight, alignment: .leading)
          .offset(x: layout.gutter, y: layout.top)
        appRow
          .frame(width: layout.rowWidth, height: layout.rowHeight)
          .offset(x: layout.gutter, y: layout.rowY)
        BackButton(action: { model.onBack() })
          .frame(width: 28, height: 28)
          .offset(x: (layout.gutter - 28) / 2 + 1, y: layout.rowY + (layout.rowHeight - 28) / 2)
          .opacity(model.phase == .granted ? 0 : 1)
      }
      .frame(width: layout.cardSize.width, height: layout.cardSize.height, alignment: .topLeading)
      .scaleEffect(model.contentScale)
      .blur(radius: model.contentBlur)
      .opacity(model.contentOpacity)
      .animation(.spring(duration: 0.3, bounce: 0), value: model.phase)
      .animation(.spring(duration: 0.25, bounce: 0), value: model.hovering)
      .animation(.spring(duration: 0.25, bounce: 0), value: model.dragging)
    }

    private var instruction: some View {
      HStack(spacing: 0) {
        ZStack {
          Image(systemName: "arrow.up")
            .font(.system(size: 18, weight: .bold))
            .foregroundStyle(Color.accentColor.gradient)
            .opacity(model.phase == .granted ? 0 : 1)
            .scaleEffect(model.phase == .granted ? 0.6 : 1)
          Image(systemName: "checkmark.circle.fill")
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(.white, Color.green)
            .opacity(model.phase == .granted ? 1 : 0)
            .scaleEffect(model.phase == .granted ? 1 : 0.6)
        }
        .frame(width: layout.icon)
        .padding(.leading, layout.iconLeading)
        .padding(.trailing, layout.textLeading - layout.iconLeading - layout.icon)
        ZStack(alignment: .leading) {
          dragText.opacity(model.phase == .granted ? 0 : 1)
            .blur(radius: model.phase == .granted ? 3 : 0)
          (Text(appName).foregroundStyle(.primary).fontWeight(.medium) + Text(" is allowed").foregroundStyle(.secondary))
            .opacity(model.phase == .granted ? 1 : 0)
            .blur(radius: model.phase == .granted ? 0 : 3)
        }
        .font(.system(size: 13))
        .lineLimit(1)
        .minimumScaleFactor(0.85)
        Spacer(minLength: 0)
      }
    }

    private var dragText: Text {
      Text("Drag ").foregroundStyle(.secondary)
        + Text(appName).foregroundStyle(.primary).fontWeight(.medium)
        + Text(" to the list above to allow ").foregroundStyle(.secondary)
        + Text(kind.title).foregroundStyle(.primary).fontWeight(.medium)
    }

    private var appRow: some View {
      let shape = RoundedRectangle(cornerRadius: layout.rowRadius, style: .continuous)
      let lifted = model.dragging
      return HStack(spacing: 0) {
        Image(nsImage: icon)
          .resizable()
          .interpolation(.high)
          .frame(width: layout.icon, height: layout.icon)
          .padding(.leading, layout.iconLeading)
          .padding(.trailing, layout.textLeading - layout.iconLeading - layout.icon)
        Text(appName)
          .font(.system(size: 13))
          .foregroundStyle(.primary)
        Spacer(minLength: 0)
      }
      .opacity(lifted ? 0.28 : 1)
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .background(shape.fill(Color.primary.opacity(model.hovering && !lifted ? 0.1 : 0.065)))
      .overlay(shape.strokeBorder(Color.primary.opacity(0.08), lineWidth: 1))
      .overlay {
        if interactive {
          DragHost(make: dragView)
        }
      }
      .opacity(model.phase == .granted ? 0.5 : 1)
    }
  }

  struct BackButton: View {
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
      Button(action: action) {
        Image(systemName: "chevron.left")
          .font(.system(size: 12, weight: .semibold))
          .foregroundStyle(hovering ? Color.primary : Color.secondary)
          .frame(width: 28, height: 28)
          .background(Circle().fill(Color.primary.opacity(hovering ? 0.12 : 0.07)))
          .contentShape(Circle())
      }
      .buttonStyle(.plain)
      .help("Back")
      .onHover { hovering = $0 }
      .animation(.spring(duration: 0.25, bounce: 0), value: hovering)
    }
  }

  struct DragHost: NSViewRepresentable {
    let make: () -> NSView
    func makeNSView(context _: Context) -> NSView { make() }
    func updateNSView(_: NSView, context _: Context) {}
  }

  /// The draggable app row's hit area. SwiftUI draws the row; this view is the NSDraggingSource,
  /// and its pasteboard item is the app bundle's file URL.
  final class DragSource: NSView, NSDraggingSource {
    let appURL: URL
    let appName: String
    let icon: NSImage
    let model: CardModel
    var onEnded: (NSDragOperation) -> Void = { _ in }
    private var down: NSEvent?

    init(appURL: URL, appName: String, icon: NSImage, model: CardModel) {
      self.appURL = appURL
      self.appName = appName
      self.icon = icon
      self.model = model
      super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError() }

    override func acceptsFirstMouse(for _: NSEvent?) -> Bool { true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .openHand) }

    override func updateTrackingAreas() {
      super.updateTrackingAreas()
      trackingAreas.forEach(removeTrackingArea)
      addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with _: NSEvent) { model.hovering = true }
    override func mouseExited(with _: NSEvent) { model.hovering = false }
    override func mouseDown(with event: NSEvent) { down = event }
    override func mouseUp(with _: NSEvent) { down = nil }

    override func mouseDragged(with event: NSEvent) {
      guard let down else { return }
      let distance = hypot(event.locationInWindow.x - down.locationInWindow.x, event.locationInWindow.y - down.locationInWindow.y)
      guard distance >= 3 else { return }
      self.down = nil
      let item = NSDraggingItem(pasteboardWriter: appURL as NSURL)
      item.setDraggingFrame(bounds, contents: dragImage())
      let session = beginDraggingSession(with: [item], event: down, source: self)
      session.animatesToStartingPositionsOnCancelOrFail = true
      session.draggingFormation = .none
      model.dragging = true
    }

    /// The row as the list will show it: opaque, so it reads over any window it crosses.
    private func dragImage() -> NSImage {
      let size = bounds.size
      let appearance = effectiveAppearance
      let name = appName
      let icon = icon
      return NSImage(size: size, flipped: false) { rect in
        appearance.performAsCurrentDrawingAppearance {
          let path = NSBezierPath(roundedRect: rect.insetBy(dx: 0.5, dy: 0.5), xRadius: 9, yRadius: 9)
          NSColor.windowBackgroundColor.blended(withFraction: 0.06, of: .labelColor)?.withAlphaComponent(0.96).setFill()
          path.fill()
          NSColor.labelColor.withAlphaComponent(0.12).setStroke()
          path.lineWidth = 1
          path.stroke()
          icon.draw(in: NSRect(x: 10, y: (size.height - 26) / 2, width: 26, height: 26))
          let font = NSFont.systemFont(ofSize: 13)
          let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.labelColor]
          let textHeight = (name as NSString).size(withAttributes: attributes).height
          (name as NSString).draw(at: NSPoint(x: 46, y: (size.height - textHeight) / 2), withAttributes: attributes)
        }
        return true
      }
    }

    func draggingSession(_: NSDraggingSession, sourceOperationMaskFor _: NSDraggingContext) -> NSDragOperation { .copy }

    func draggingSession(_: NSDraggingSession, endedAt _: NSPoint, operation: NSDragOperation) {
      model.dragging = false
      model.hovering = false
      onEnded(operation)
    }
  }

  /// Frosts the glass so list rows behind the card don't read through its text.
  final class Frost: NSView {
    override var wantsUpdateLayer: Bool { true }
    override func updateLayer() {
      layer?.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.62).cgColor
    }
  }

  /// One card surface: a two-layer shadow, frosted Liquid Glass, and the content clipped to it.
  /// The flight and the docked panel use the same class so the hand-off is pixel for pixel.
  final class CardSurface: NSView {
    let glass = CardSurface.makeGlass()
    let clip = NSView()
    let frost = Frost()
    let content: NSView
    let contentSize: NSSize
    private let contact = CALayer()
    private let ambient = CALayer()

    init(content: NSView, size: NSSize) {
      self.content = content
      self.contentSize = size
      super.init(frame: NSRect(origin: .zero, size: size))
      wantsLayer = true
      layer?.masksToBounds = false
      for shadow in [ambient, contact] {
        shadow.shadowColor = NSColor.black.cgColor
        shadow.shadowOffset = .zero
        layer?.addSublayer(shadow)
      }
      contact.shadowRadius = 1.5
      contact.shadowOffset = CGSize(width: 0, height: -0.5)
      ambient.shadowOffset = CGSize(width: 0, height: -8)
      addSubview(glass)
      clip.wantsLayer = true
      clip.layer?.masksToBounds = true
      clip.layer?.cornerCurve = .continuous
      addSubview(clip)
      frost.wantsLayer = true
      frost.autoresizingMask = [.width, .height]
      clip.addSubview(frost)
      content.frame = NSRect(origin: .zero, size: size)
      clip.addSubview(content)
      apply(size: size, radius: 18, lift: 0)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError() }

    /// Liquid Glass on macOS 26 (when built with its SDK); a frosted popover material before that.
    static func makeGlass() -> NSView {
      #if compiler(>=6.2)
      if #available(macOS 26, *) {
        let glass = NSGlassEffectView()
        glass.style = .regular
        return glass
      }
      #endif
      let material = NSVisualEffectView()
      material.material = .popover
      material.blendingMode = .behindWindow
      material.state = .active
      material.wantsLayer = true
      material.layer?.masksToBounds = true
      material.layer?.cornerCurve = .continuous
      return material
    }

    static func setCornerRadius(_ radius: CGFloat, of glass: NSView) {
      #if compiler(>=6.2)
      if #available(macOS 26, *), let glass = glass as? NSGlassEffectView {
        glass.cornerRadius = radius
        return
      }
      #endif
      glass.layer?.cornerRadius = radius
    }

    /// `lift` 0 is docked; 1 is the top of the arc, with a deeper, softer shadow.
    func apply(size: NSSize, radius: CGFloat, lift: CGFloat) {
      let bounds = NSRect(origin: .zero, size: size)
      glass.frame = bounds
      CardSurface.setCornerRadius(radius, of: glass)
      clip.frame = bounds
      clip.layer?.cornerRadius = radius
      frost.frame = bounds
      content.frame.origin = NSPoint(x: (size.width - contentSize.width) / 2, y: (size.height - contentSize.height) / 2)
      let path = CGPath(roundedRect: bounds, cornerWidth: min(radius, size.width / 2), cornerHeight: min(radius, size.height / 2), transform: nil)
      for shadow in [ambient, contact] {
        shadow.frame = bounds
        shadow.shadowPath = path
      }
      contact.shadowOpacity = Float(0.3 - 0.1 * lift)
      ambient.shadowOpacity = Float(0.32 + 0.14 * lift)
      ambient.shadowRadius = 18 + 14 * lift
      ambient.shadowOffset = CGSize(width: 0, height: -8 - 12 * lift)
    }
  }

  /// Borderless, non-activating, on every Space (Stage Manager moves ordinary windows aside).
  final class OverlayPanel: NSPanel {
    init(frame: NSRect, level: NSWindow.Level) {
      super.init(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel, .fullSizeContentView], backing: .buffered, defer: false)
      isFloatingPanel = true
      self.level = level
      collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
      isOpaque = false
      backgroundColor = .clear
      hasShadow = false
      hidesOnDeactivate = false
      isReleasedWhenClosed = false
      becomesKeyOnlyIfNeeded = true
      animationBehavior = .none
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// The flight's window spans the arc, which can rise past the menu bar. AppKit would push
    /// it down onto the screen, and the card with it.
    override func constrainFrameRect(_ frameRect: NSRect, to _: NSScreen?) -> NSRect { frameRect }
  }
}

// MARK: - Settings window

extension PermissionPanel {
  /// Follows the System Settings window through the window server (no Accessibility needed).
  /// Matches its owner by bundle id, so a localized "System Settings" name doesn't matter.
  struct SettingsWindow {
    static func pids() -> Set<pid_t> {
      Set(NSRunningApplication.runningApplications(withBundleIdentifier: PermissionPanel.settingsBundleID).map(\.processIdentifier))
    }

    static func windows() -> [[String: Any]] {
      let pids = pids()
      guard !pids.isEmpty,
            let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]
      else { return [] }
      return list.filter { ($0[kCGWindowOwnerPID as String] as? pid_t).map(pids.contains) ?? false }
    }

    /// The largest ordinary Settings window: its number and global top-left bounds.
    static func main() -> (id: CGWindowID, frame: CGRect)? {
      var best: (id: CGWindowID, frame: CGRect)?
      for window in windows() {
        guard (window[kCGWindowLayer as String] as? Int) == 0,
              let id = window[kCGWindowNumber as String] as? CGWindowID,
              let bounds = window[kCGWindowBounds as String] as? NSDictionary,
              let frame = CGRect(dictionaryRepresentation: bounds),
              frame.width >= 300, frame.height >= 200
        else { continue }
        if best.map({ frame.width * frame.height > $0.frame.width * $0.frame.height }) ?? true {
          best = (id, frame)
        }
      }
      return best
    }

    /// One window's current bounds, cheaper than listing every window.
    static func frame(of id: CGWindowID) -> CGRect? {
      let ids = [UnsafeRawPointer(bitPattern: UInt(id))] as CFArray
      guard let info = CGWindowListCreateDescriptionFromArray(ids) as? [[String: Any]],
            let window = info.first,
            (window[kCGWindowIsOnscreen as String] as? Bool) ?? false,
            let bounds = window[kCGWindowBounds as String] as? NSDictionary
      else { return nil }
      return CGRect(dictionaryRepresentation: bounds)
    }

    /// Sheets and alerts (the Quit & Reopen prompt) add windows; their count dropping back means
    /// one was dismissed.
    static func count() -> Int { windows().count }
  }
}

// MARK: - Controller

extension PermissionPanel {
  final class Controller: NSObject, NSApplicationDelegate {
    let kind: Kind
    let sourceQuartz: CGRect?
    let appURL: URL
    let preview: Bool
    let reduceMotion: Bool
    let appName: String
    let icon: NSImage

    private var layout = Layout(settingsWidth: 740)
    private var panel: OverlayPanel?
    private var panelSurface: CardSurface?
    private var panelModel = CardModel()
    private var flight: Flight?
    private var settingsID: CGWindowID?
    private var settingsFrame: CGRect?
    private var missingSince: Date?
    private var trackTimer: Timer?
    private var pollTimer: Timer?
    private var fastPollUntil = Date.distantPast
    private var checking = false
    private var dropRace: (deadline: Date, peakWindows: Int)?
    private var finished = false
    private let startedAt = Date()

    init(kind: Kind, sourceQuartz: CGRect?, appURL: URL, preview: Bool, reduceMotion: Bool) {
      self.kind = kind
      self.sourceQuartz = sourceQuartz
      self.appURL = appURL
      self.preview = preview
      self.reduceMotion = reduceMotion
      appName = FileManager.default.displayName(atPath: appURL.path).replacingOccurrences(of: ".app", with: "")
      let icon = NSWorkspace.shared.icon(forFile: appURL.path)
      icon.size = NSSize(width: 64, height: 64)
      self.icon = icon
    }

    func applicationDidFinishLaunching(_: Notification) {
      listenToStdin()
      DistributedNotificationCenter.default().addObserver(
        forName: NSNotification.Name("com.apple.TCC.updated"), object: nil, queue: .main
      ) { [weak self] _ in
        self?.fastPollUntil = Date().addingTimeInterval(5)
        self?.poll()
      }
      if preview {
        open()
        return
      }
      kind.checkFresh { [weak self] granted in
        guard let self else { return }
        if granted { self.finish(granted: true, reason: nil, animated: false) } else { self.open() }
      }
    }

    // MARK: Flow

    private func open() {
      NSWorkspace.shared.open(kind.settingsURL)
      waitForSettings(deadline: Date().addingTimeInterval(6), firstSeen: nil, last: nil)
    }

    /// Waits for the Settings window to stop moving: two equal samples 100 ms apart, or 1.2 s
    /// after it first appeared (a cold launch, or the genie from the Dock).
    private func waitForSettings(deadline: Date, firstSeen: Date?, last: CGRect?) {
      guard !finished else { return }
      let now = Date()
      if let window = SettingsWindow.main() {
        if let last, window.frame.integral == last.integral {
          return dock(window.id, window.frame)
        }
        if let firstSeen, now.timeIntervalSince(firstSeen) >= 1.2 {
          return dock(window.id, window.frame)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
          self.waitForSettings(deadline: deadline, firstSeen: firstSeen ?? now, last: window.frame)
        }
        return
      }
      if now > deadline { return finish(granted: false, reason: "no-settings-window", animated: false) }
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
        self.waitForSettings(deadline: deadline, firstSeen: nil, last: nil)
      }
    }

    private func dock(_ id: CGWindowID, _ frame: CGRect) {
      settingsID = id
      settingsFrame = frame
      layout = Layout(settingsWidth: frame.width)
      let target = dockFrame(settingsQuartz: frame, layout: layout)
      buildPanel(at: target)
      if reduceMotion || sourceQuartz == nil {
        fadeIn()
      } else {
        fly(from: appKit(fromQuartz: sourceQuartz!), to: target)
      }
    }

    private func docked() {
      var event: [String: Any] = ["event": "docked", "permission": kind.rawValue]
      if let settingsFrame, let panel {
        // Where it went, in CGWindow coordinates, for checking placement in a log.
        let card = quartz(fromAppKit: panel.frame.insetBy(dx: Controller.margin, dy: Controller.margin))
        let settings = appKit(fromQuartz: settingsFrame)
        let screen = NSScreen.screens.first { $0.frame.intersects(settings) } ?? NSScreen.main
        event["dock"] = PermissionPanel.dock(settings: settings, visible: screen?.visibleFrame ?? settings, layout: layout).dock.rawValue
        event["card"] = [card.minX, card.minY, card.width, card.height].map { Double($0) }
        event["settings"] = [settingsFrame.minX, settingsFrame.minY, settingsFrame.width, settingsFrame.height].map { Double($0) }
      }
      emit(event)
      startTracking()
      startPolling()
    }

    // MARK: Panel

    private func makeSurface(model: CardModel, interactive: Bool) -> CardSurface {
      let dragSource = DragSource(appURL: appURL, appName: appName, icon: icon, model: model)
      dragSource.onEnded = { [weak self] operation in self?.dragEnded(operation) }
      let content = CardContent(
        model: model, kind: kind, layout: layout, appName: appName, icon: icon,
        interactive: interactive, dragView: { dragSource }
      )
      let host = NSHostingView(rootView: content)
      host.sizingOptions = []
      return CardSurface(content: host, size: layout.cardSize)
    }

    static let margin: CGFloat = 48

    private func buildPanel(at card: NSRect) {
      panelModel.onBack = { [weak self] in self?.cancel(reason: "back") }
      let surface = makeSurface(model: panelModel, interactive: true)
      let frame = card.insetBy(dx: -Controller.margin, dy: -Controller.margin)
      let panel = OverlayPanel(frame: frame, level: .floating)
      let root = NSView(frame: NSRect(origin: .zero, size: frame.size))
      surface.frame.origin = NSPoint(x: Controller.margin, y: Controller.margin)
      root.addSubview(surface)
      panel.contentView = root
      self.panel = panel
      panelSurface = surface
    }

    private func moveDock(to card: NSRect) {
      panel?.setFrame(card.insetBy(dx: -Controller.margin, dy: -Controller.margin), display: true)
    }

    private func fadeIn() {
      guard let panel else { return }
      panel.alphaValue = 0
      panel.orderFrontRegardless()
      NSAnimationContext.runAnimationGroup { context in
        context.duration = fadeDuration
        panel.animator().alphaValue = 1
      }
      docked()
    }

    // MARK: Flight

    private func fly(from source: NSRect, to target: NSRect) {
      let model = CardModel()
      model.contentOpacity = 0
      let surface = makeSurface(model: model, interactive: false)
      let flight = Flight(source: source, target: target, spring: flightSpring, surface: surface, model: model, reverse: false)
      self.flight = flight
      flight.start { [weak self] in
        guard let self, let panel = self.panel else { return }
        panel.orderFrontRegardless()
        // The overlay's card sits exactly over the panel's; drop it once the panel has drawn.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
          flight.close()
          self.flight = nil
        }
        self.docked()
      }
    }

    private func flyBack(then done: @escaping () -> Void) {
      guard let panel, let surface = panelSurface else { return done() }
      let card = NSRect(origin: NSPoint(x: panel.frame.minX + Controller.margin, y: panel.frame.minY + Controller.margin), size: surface.frame.size)
      guard !reduceMotion, let sourceQuartz else { return fadeOut(then: done) }
      let model = CardModel()
      model.phase = panelModel.phase
      let flyer = makeSurface(model: model, interactive: false)
      let flight = Flight(source: appKit(fromQuartz: sourceQuartz), target: card, spring: returnSpring, surface: flyer, model: model, reverse: true)
      self.flight = flight
      flight.onFirstFrame = { panel.orderOut(nil) }
      flight.start { [weak self] in
        flight.close()
        self?.flight = nil
        done()
      }
    }

    private func fadeOut(then done: @escaping () -> Void) {
      guard let panel, panel.isVisible else { return done() }
      NSAnimationContext.runAnimationGroup { context in
        context.duration = fadeDuration * 0.8
        panel.animator().alphaValue = 0
      } completionHandler: {
        panel.orderOut(nil)
        done()
      }
      if !reduceMotion {
        withAnimation(.spring(duration: fadeDuration * 0.8, bounce: 0)) {
          panelModel.contentScale = 0.96
          panelModel.contentBlur = 3
        }
      }
    }

    // MARK: Tracking

    private func startTracking() {
      trackTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { [weak self] _ in self?.track() }
      RunLoop.main.add(trackTimer!, forMode: .common)
    }

    /// The panel follows Settings 1:1 (the user is moving it) and steps aside while another app
    /// is in front.
    private func track() {
      guard !finished, let panel else { return }
      var frame = settingsID.flatMap(SettingsWindow.frame(of:))
      if frame == nil, let window = SettingsWindow.main() {
        settingsID = window.id
        frame = window.frame
      }
      guard let frame else {
        missingSince = missingSince ?? Date()
        if Date().timeIntervalSince(missingSince!) > 0.6 { cancel(reason: "settings-closed") }
        return
      }
      missingSince = nil
      if frame != settingsFrame {
        settingsFrame = frame
        moveDock(to: dockFrame(settingsQuartz: frame, layout: layout))
      }
      if dropRace == nil, flight == nil {
        let front = NSWorkspace.shared.frontmostApplication
        let ours = front?.bundleIdentifier == settingsBundleID
          || front?.processIdentifier == ProcessInfo.processInfo.processIdentifier
          || front.map(isScrawl) ?? false
          || panelModel.dragging
        let alpha: CGFloat = ours ? 1 : 0
        if panel.alphaValue != alpha, !isFading {
          isFading = true
          NSAnimationContext.runAnimationGroup { context in
            context.duration = fadeDuration
            panel.animator().alphaValue = alpha
          } completionHandler: { self.isFading = false }
        }
      }
      if Date().timeIntervalSince(startedAt) > 600 { cancel(reason: "timeout") }
    }

    private var isFading = false

    /// Scrawl itself or one of its helpers, such as the onboarding window.
    private func isScrawl(_ app: NSRunningApplication) -> Bool {
      var url = app.executableURL?.resolvingSymlinksInPath() ?? app.bundleURL
      while let current = url, current.path != "/" {
        if current.pathExtension.lowercased() == "app" {
          return current.standardizedFileURL.path == appURL.standardizedFileURL.path
        }
        url = current.deletingLastPathComponent()
      }
      return false
    }

    // MARK: Grant

    private func startPolling() {
      guard !preview else { return }
      pollTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in self?.tickPoll() }
    }

    private var lastPoll = Date.distantPast

    private func tickPoll() {
      let interval = Date() < fastPollUntil || dropRace != nil ? 0.25 : 0.75
      if Date().timeIntervalSince(lastPoll) >= interval { poll() }
      guard let race = dropRace else { return }
      // The race ends when a prompt Settings showed is dismissed, or after 10 s.
      let count = SettingsWindow.count()
      if count > race.peakWindows { dropRace = (race.deadline, count) }
      if count < race.peakWindows || Date() > race.deadline {
        dropRace = nil
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { self.poll(thenReshow: true) }
      }
    }

    private func poll(thenReshow: Bool = false) {
      guard !finished, !preview, !checking else { return }
      checking = true
      lastPoll = Date()
      kind.checkFresh { [weak self] granted in
        guard let self else { return }
        self.checking = false
        if granted { return self.finish(granted: true, reason: nil, animated: true) }
        if thenReshow, self.dropRace == nil { self.reshow() }
      }
    }

    /// A drop is a hint, never the answer: step out of the way of whatever Settings shows, read
    /// the grant fresh, and come back if it isn't there.
    private func dragEnded(_ operation: NSDragOperation) {
      guard !finished, !operation.isEmpty else { return }
      emit(["event": "dropped", "permission": kind.rawValue])
      if preview { return }
      dropRace = (Date().addingTimeInterval(10), SettingsWindow.count())
      fastPollUntil = Date().addingTimeInterval(10)
      guard let panel else { return }
      panelModel.phase = .waiting
      NSAnimationContext.runAnimationGroup { context in
        context.duration = fadeDuration
        panel.animator().alphaValue = 0
      }
      poll()
    }

    private func reshow() {
      guard !finished, let panel else { return }
      panelModel.phase = .instruction
      NSAnimationContext.runAnimationGroup { context in
        context.duration = fadeDuration
        panel.animator().alphaValue = 1
      }
    }

    // MARK: Ending

    private func listenToStdin() {
      var info = stat()
      guard fstat(0, &info) == 0, (info.st_mode & S_IFMT) == S_IFIFO || (info.st_mode & S_IFMT) == S_IFSOCK else { return }
      FileHandle.standardInput.readabilityHandler = { [weak self] handle in
        let data = handle.availableData
        let text = String(decoding: data, as: UTF8.self)
        if text.contains("granted") {
          handle.readabilityHandler = nil
          DispatchQueue.main.async { self?.finish(granted: true, reason: nil, animated: true) }
        } else if text.contains("back") {
          handle.readabilityHandler = nil
          DispatchQueue.main.async { self?.cancel(reason: "back") }
        } else if data.isEmpty || text.contains("cancel") {
          handle.readabilityHandler = nil
          DispatchQueue.main.async { self?.cancel(reason: "parent") }
        }
      }
    }

    private func cancel(reason: String) {
      guard !finished else { return }
      finished = true
      stopTimers()
      emit(["event": "cancelled", "permission": kind.rawValue, "reason": reason])
      if reason == "back" {
        flyBack { exit(0) }
      } else {
        fadeOut { exit(0) }
      }
    }

    private func finish(granted: Bool, reason: String?, animated: Bool) {
      guard !finished else { return }
      finished = true
      stopTimers()
      var event: [String: Any] = ["event": granted ? "granted" : "cancelled", "permission": kind.rawValue]
      if let reason { event["reason"] = reason }
      emit(event)
      guard animated, let panel else { exit(0) }
      panelModel.phase = .granted
      if panel.alphaValue < 1 {
        NSAnimationContext.runAnimationGroup { context in
          context.duration = fadeDuration
          panel.animator().alphaValue = 1
        }
      }
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.9) {
        self.fadeOut { exit(0) }
      }
    }

    private func stopTimers() {
      trackTimer?.invalidate()
      pollTimer?.invalidate()
    }
  }
}

// MARK: - Flight

extension PermissionPanel {
  /// The card in the air: one overlay window over the whole path, the card moved inside it on
  /// the display's clock. The path is a quadratic Bézier through a point above both ends; the
  /// size runs ahead of the path (width first), the corners go from the button's capsule to the
  /// card's, and the content resolves (opacity, 0.96 → 1 scale, 3 px → 0 blur) as the card
  /// reaches full size.
  final class Flight: NSObject {
    let source: NSRect
    let target: NSRect
    let spring: Spring
    let surface: CardSurface
    let model: CardModel
    let reverse: Bool
    /// The overlay the card flies in; nil when a caller hosts the surface itself (renders).
    let window: OverlayPanel?
    /// Screen position of the host view's origin.
    let hostOrigin: NSPoint
    var onFirstFrame: () -> Void = {}
    private var done: () -> Void = {}
    private var link: CADisplayLink?
    private var startTime: CFTimeInterval = 0
    private let control: CGPoint

    /// Flies in its own overlay window spanning the path.
    convenience init(source: NSRect, target: NSRect, spring: Spring, surface: CardSurface, model: CardModel, reverse: Bool) {
      let control = Flight.control(source: source, target: target)
      let pad: CGFloat = 64
      let bounds = source.union(target).union(NSRect(x: control.x, y: control.y, width: 1, height: 1)).insetBy(dx: -pad, dy: -pad)
      let window = OverlayPanel(frame: bounds, level: NSWindow.Level(rawValue: NSWindow.Level.floating.rawValue + 1))
      window.ignoresMouseEvents = true
      let root = NSView(frame: NSRect(origin: .zero, size: bounds.size))
      window.contentView = root
      self.init(source: source, target: target, spring: spring, surface: surface, model: model, reverse: reverse, host: root, hostOrigin: bounds.origin, window: window)
    }

    init(source: NSRect, target: NSRect, spring: Spring, surface: CardSurface, model: CardModel, reverse: Bool, host: NSView, hostOrigin: NSPoint, window: OverlayPanel? = nil) {
      self.source = source
      self.target = target
      self.spring = spring
      self.surface = surface
      self.model = model
      self.reverse = reverse
      self.window = window
      self.hostOrigin = hostOrigin
      control = Flight.control(source: source, target: target)
      host.addSubview(surface)
      super.init()
    }

    /// The Bézier's control point: midway across, `arcHeight` above the higher end (less on
    /// short hops).
    static func control(source: NSRect, target: NSRect) -> CGPoint {
      let distance = hypot(target.midX - source.midX, target.midY - source.midY)
      let lift = min(PermissionPanel.arcHeight, distance * PermissionPanel.arcShare)
      let top = max(source.midY, target.midY)
      // The card's centre stays far enough below the top of the screen for the card to fit.
      let screen = NSScreen.screens.first { $0.frame.contains(NSPoint(x: source.midX, y: source.midY)) } ?? NSScreen.main
      let ceiling = (screen?.visibleFrame.maxY ?? .greatestFiniteMagnitude) - target.height / 2 - 8
      return CGPoint(x: (source.midX + target.midX) / 2, y: max(top, min(top + lift, ceiling)))
    }

    func start(done: @escaping () -> Void) {
      self.done = done
      step(at: 0)
      window?.orderFrontRegardless()
      onFirstFrame()
      startTime = CACurrentMediaTime()
      let link = surface.displayLink(target: self, selector: #selector(frame(_:)))
      link.add(to: .main, forMode: .common)
      self.link = link
    }

    func close() {
      link?.invalidate()
      link = nil
      window?.orderOut(nil)
    }

    @objc private func frame(_ link: CADisplayLink) {
      let t = link.targetTimestamp - startTime
      if t >= spring.settleTime {
        step(at: spring.settleTime)
        link.invalidate()
        self.link = nil
        done()
        return
      }
      step(at: t)
    }

    func step(at t: Double) {
      // Progress from the source (0) to the dock (1); reversed flights run it backwards.
      let path = spring.value(t)
      let width = Spring(duration: spring.duration * PermissionPanel.widthLead, bounce: spring.bounce).value(t)
      let p = reverse ? 1 - path : path
      let w = reverse ? 1 - width : width
      let h = p
      let start = CGPoint(x: source.midX, y: source.midY)
      let end = CGPoint(x: target.midX, y: target.midY)
      let u = 1 - p
      let center = CGPoint(
        x: u * u * start.x + 2 * u * p * control.x + p * p * end.x,
        y: u * u * start.y + 2 * u * p * control.y + p * p * end.y
      )
      let size = NSSize(
        width: source.width + (target.width - source.width) * w,
        height: source.height + (target.height - source.height) * h
      )
      let startRadius = source.height / 2
      let radius = startRadius + (18 - startRadius) * min(w, h)
      let origin = hostOrigin
      CATransaction.begin()
      CATransaction.setDisableActions(true)
      surface.frame = NSRect(x: center.x - size.width / 2 - origin.x, y: center.y - size.height / 2 - origin.y, width: size.width, height: size.height)
      surface.apply(size: size, radius: radius, lift: 4 * p * (1 - p))
      // Lift off the button over the first stretch; land on it fading on the way back.
      surface.alphaValue = smoothstep(0, 0.14, p)
      let resolve = smoothstep(reverse ? 0.5 : 0.3, reverse ? 0.9 : 0.85, min(w, h))
      var transaction = Transaction()
      transaction.disablesAnimations = true
      withTransaction(transaction) {
        model.contentOpacity = resolve
        model.contentScale = 0.96 + 0.04 * resolve
        model.contentBlur = 3 * (1 - resolve)
      }
      CATransaction.commit()
    }

    private func smoothstep(_ a: Double, _ b: Double, _ x: Double) -> Double {
      let t = min(max((x - a) / (b - a), 0), 1)
      return t * t * (3 - 2 * t)
    }
  }
}

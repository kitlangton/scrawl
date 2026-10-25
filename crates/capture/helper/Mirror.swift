@preconcurrency import AVFoundation
import AppKit
import CoreMediaIO
import IOKit
import QuartzCore

/// Cabled iPhones and iPads, which macOS publishes as muxed capture devices once an app sets
/// `kCMIOHardwarePropertyAllowScreenCaptureDevices` (the switch QuickTime flips for New Movie
/// Recording). A phone that was plugged in before then appears about a second later with no
/// connect notification, so callers watch the device list rather than wait for one.
enum Phone {
  static func publishScreenCaptureDevices() {
    var address = CMIOObjectPropertyAddress(
      mSelector: CMIOObjectPropertySelector(kCMIOHardwarePropertyAllowScreenCaptureDevices),
      mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
      mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain)
    )
    var allow: UInt32 = 1
    CMIOObjectSetPropertyData(CMIOObjectID(kCMIOObjectSystemObject), &address, 0, nil, UInt32(MemoryLayout<UInt32>.size), &allow)
  }

  static var devices: [AVCaptureDevice] {
    let muxed = AVCaptureDevice.DiscoverySession(deviceTypes: [.external], mediaType: .muxed, position: .unspecified).devices
    let filtered = muxed.filter { $0.modelID == "iOS Device" }
    return filtered.isEmpty ? muxed : filtered
  }

  static func normalizeID(_ id: String) -> String {
    id.replacingOccurrences(of: "-", with: "").lowercased()
  }

  /// Cabled iPhones and iPads from the USB registry: instant, and unlike AVFoundation it needs no
  /// camera permission, so the picker can look every time it opens without prompting.
  static func cabled() -> [[String: Any]] {
    var iterator: io_iterator_t = 0
    guard IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOUSBHostDevice"), &iterator) == KERN_SUCCESS else { return [] }
    defer { IOObjectRelease(iterator) }
    var found: [[String: Any]] = []
    while case let service = IOIteratorNext(iterator), service != 0 {
      defer { IOObjectRelease(service) }
      func property(_ key: String) -> Any? {
        IORegistryEntryCreateCFProperty(service, key as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue()
      }
      let vendor = (property("idVendor") ?? property("kUSBVendorID")) as? Int
      guard vendor == 0x05ac else { continue }
      var entryName = [CChar](repeating: 0, count: 128)
      IORegistryEntryGetName(service, &entryName)
      let product = (property("USB Product Name") ?? property("kUSBProductString")) as? String ?? String(cString: entryName)
      guard product.hasPrefix("iPhone") || product.hasPrefix("iPad") else { continue }
      let serial = normalizeID((property("USB Serial Number") ?? property("kUSBSerialNumberString")) as? String ?? "")
      found.append(["id": serial, "name": product, "model": product, "width": 0, "height": 0])
    }
    return found
  }

  static func size(of device: AVCaptureDevice) -> CMVideoDimensions {
    CMVideoFormatDescriptionGetDimensions(device.activeFormat.formatDescription)
  }

  static func describe(_ device: AVCaptureDevice) -> [String: Any] {
    let size = size(of: device)
    return ["id": device.uniqueID, "name": device.localizedName, "model": device.modelID, "width": Int(size.width), "height": Int(size.height)]
  }
}

/// Mirrors a phone in a floating window and records its native video with the Mac's microphone.
///
/// Events on stdout (one JSON object per line):
///   {"event":"device", id, name, model, width, height}   attached; repeated when the phone rotates
///   {"event":"rect", x, y, width, height}                the picture in global top-left points
///   {"event":"recording", "startedAt": epochMs}          the movie's first frame
///   {"event":"marker", "at": epochMs}                    M pressed in the mirror window
///   {"event":"finished", path, durationMs}               movie finalized; the helper then exits
///   {"event":"cancelled"} / {"event":"error", message}
///   {"event":"warning", message}                         e.g. no microphone, so no narration
/// Commands on stdin: `show`, `hide`, `start [PATH]`, `stop`, `cancel`. End of input means stop.
final class Mirror: NSObject, NSApplicationDelegate, NSWindowDelegate, AVCaptureFileOutputRecordingDelegate {
  static func run(output: URL?, previewOnly: Bool = false, hiddenPreview: Bool = false, deviceID: String?, microphone: (id: String, name: String)? = nil) -> Never {
    let app = NSApplication.shared
    let mirror = Mirror(output: output, previewOnly: previewOnly, hiddenPreview: hiddenPreview, deviceID: deviceID)
    mirror.microphone = microphone
    app.setActivationPolicy(.accessory)
    app.delegate = mirror
    withExtendedLifetime(mirror) { app.run() }
    exit(0)
  }

  var output: URL?
  var previewOnly: Bool
  var hiddenPreview: Bool
  let deviceID: String?
  /// The microphone chosen in Settings (CoreAudio UID, which is `AVCaptureDevice.uniqueID`).
  var microphone: (id: String, name: String)?
  lazy var session = AVCaptureSession()
  lazy var movieOutput = AVCaptureMovieFileOutput()
  let queue = DispatchQueue(label: "dev.kitlangton.scrawl.helper.session")
  var phoneInput: AVCaptureDeviceInput?
  var window: MirrorPanel?
  var caseView: PhoneCaseView?
  var discoveryObservation: NSKeyValueObservation?
  var aspect: CGFloat = 2868.0 / 1320.0
  var stopping = false
  var cancelled = false
  var finished = false
  /// The movie output was asked to start (it starts once the phone's video is flowing).
  var movieRequested = false

  init(output: URL?, previewOnly: Bool, hiddenPreview: Bool, deviceID: String?) {
    self.output = output
    self.previewOnly = previewOnly
    self.hiddenPreview = hiddenPreview
    self.deviceID = deviceID
  }

  func applicationDidFinishLaunching(_: Notification) {
    readCommands()
    showWindow(title: "iPhone", orderFront: !hiddenPreview)
    AVCaptureDevice.requestAccess(for: .video) { granted in
      DispatchQueue.main.async {
        guard granted else { return self.die("Camera access denied; allow Scrawl in System Settings › Privacy & Security › Camera") }
        AVCaptureDevice.requestAccess(for: .audio) { micGranted in
          DispatchQueue.main.async {
            if !micGranted {
              self.warn("Microphone access denied; recording without narration (allow Scrawl in System Settings › Privacy & Security › Microphone)")
            }
            Phone.publishScreenCaptureDevices()
            let deadline = Date().addingTimeInterval(12)
            NotificationCenter.default.addObserver(forName: AVCaptureDevice.wasConnectedNotification, object: nil, queue: .main) { [weak self] _ in
              self?.findPhone(deadline: deadline)
            }
            self.findPhone(deadline: deadline)
          }
        }
      }
    }
  }

  private func findPhone(deadline: Date) {
    guard phoneInput == nil else { return }
    Phone.publishScreenCaptureDevices()
    let phones = Phone.devices
    let wanted = deviceID.map(Phone.normalizeID).flatMap { $0.isEmpty ? nil : $0 }
    if let phone = phones.first(where: { wanted == nil || Phone.normalizeID($0.uniqueID) == wanted }) ?? phones.first {
      attach(phone)
    } else if Date() < deadline, !stopping {
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { self.findPhone(deadline: deadline) }
    } else if phoneInput == nil {
      die(stopping ? "Stopped before the iPhone appeared" : "No iPhone found; connect one with a cable and unlock it")
    }
  }

  private func attach(_ device: AVCaptureDevice) {
    guard phoneInput == nil else { return }
    let input: AVCaptureDeviceInput
    do { input = try AVCaptureDeviceInput(device: device) } catch { return die("Could not open \(device.localizedName): \(error.localizedDescription)") }
    phoneInput = input
    let size = Phone.size(of: device)
    if size.width > 0, size.height > 0 { aspect = CGFloat(size.height) / CGFloat(size.width) }
    emit(["event": "device"].merging(Phone.describe(device)) { a, _ in a })
    window?.title = device.localizedName
    let session = session, movieOutput = movieOutput
    caseView?.attachSession(session)
    fitWindow()

    var mic: AVCaptureDeviceInput?
    if AVCaptureDevice.authorizationStatus(for: .audio) == .authorized {
      // The chosen microphone, else the macOS default: a missing one never costs the narration.
      let chosen = microphone.flatMap { AVCaptureDevice(uniqueID: $0.id) }
      let fallback = AVCaptureDevice.default(for: .audio)
      if let wanted = microphone, chosen == nil, let fallback {
        warn("\(wanted.name) isn't connected. Using \(fallback.localizedName).")
      }
      let candidates = [chosen, fallback].compactMap { $0 }
      var failure: Error?
      for device in candidates where mic == nil {
        do { mic = try AVCaptureDeviceInput(device: device) } catch { failure = error }
      }
      if candidates.isEmpty {
        warn("No microphone; recording without narration")
      } else if mic == nil, let failure {
        warn("Could not open the microphone (\(failure.localizedDescription)); recording without narration")
      }
    }
    queue.async {
      session.beginConfiguration()
      if session.canAddInput(input) { session.addInput(input) }
      if let mic, session.canAddInput(mic) { session.addInput(mic) }
      if session.canAddOutput(movieOutput) { session.addOutput(movieOutput) }
      Self.disablePhoneAudio(movieOutput, phone: input)
      session.commitConfiguration()
      session.startRunning()
      DispatchQueue.main.async { self.startMovieWhenReady() }
    }
    NotificationCenter.default.addObserver(forName: AVCaptureInput.Port.formatDescriptionDidChangeNotification, object: nil, queue: .main) { [weak self] note in
      guard let self, let port = note.object as? AVCaptureInput.Port, port.input === self.phoneInput,
            let format = port.formatDescription, CMFormatDescriptionGetMediaType(format) == kCMMediaType_Video else { return }
      let size = CMVideoFormatDescriptionGetDimensions(format)
      guard size.width > 0, size.height > 0, let device = self.phoneInput?.device else { return }
      let aspect = CGFloat(size.height) / CGFloat(size.width)
      guard abs(aspect - self.aspect) > 0.01 else { return }
      self.aspect = aspect
      emit(["event": "device", "id": device.uniqueID, "name": device.localizedName, "model": device.modelID, "width": Int(size.width), "height": Int(size.height)])
      self.fitWindow()
    }
    NotificationCenter.default.addObserver(forName: AVCaptureDevice.wasDisconnectedNotification, object: nil, queue: .main) { [weak self] note in
      guard let self, (note.object as? AVCaptureDevice) == self.phoneInput?.device else { return }
      self.stop()
    }
  }

  /// Narration comes from the Mac's microphone, not the phone's own sound. A muxed phone input
  /// can label its ports `.audio` or `.muxed`, so audio connections are recognized by what they
  /// carry (an audio port or audio channels) rather than by one port type.
  static func disablePhoneAudio(_ output: AVCaptureMovieFileOutput, phone: AVCaptureDeviceInput) {
    for connection in output.connections {
      let ports = connection.inputPorts
      guard ports.contains(where: { $0.input === phone }) else { continue }
      let isVideo = ports.contains { port in
        port.mediaType == .video || port.formatDescription.map { CMFormatDescriptionGetMediaType($0) == kCMMediaType_Video } == true
      }
      let isAudio = !connection.audioChannels.isEmpty || ports.contains { port in
        port.mediaType == .audio || port.formatDescription.map { CMFormatDescriptionGetMediaType($0) == kCMMediaType_Audio } == true
      }
      if isAudio, !isVideo {
        connection.isEnabled = false
      } else if isAudio {
        log("phone audio shares the video connection and cannot be dropped separately")
      }
    }
  }

  /// The phone's video size from its input ports, zero until frames have a format.
  private func phoneVideoSize() -> CMVideoDimensions? {
    phoneInput?.ports.lazy
      .compactMap(\.formatDescription)
      .filter { CMFormatDescriptionGetMediaType($0) == kCMMediaType_Video }
      .map(CMVideoFormatDescriptionGetDimensions)
      .first { $0.width > 0 && $0.height > 0 }
  }

  /// Starts the movie only once the session is running and the phone's video port reports real
  /// dimensions; starting earlier can produce a movie without a usable video track.
  private func startMovieWhenReady(deadline: Date = Date().addingTimeInterval(8)) {
    guard !previewOnly, !stopping, !movieRequested, phoneInput != nil, let output else { return }
    guard session.isRunning, phoneVideoSize() != nil else {
      if Date() < deadline {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { self.startMovieWhenReady(deadline: deadline) }
      } else {
        die("The iPhone's video never started; unlock the phone and try again")
      }
      return
    }
    movieRequested = true
    try? FileManager.default.removeItem(at: output)
    let movieOutput = movieOutput
    queue.async {
      if !movieOutput.isRecording { movieOutput.startRecording(to: output, recordingDelegate: self) }
    }
  }

  private func warn(_ message: String) {
    log(message)
    emit(["event": "warning", "message": message])
  }

  // MARK: Window

  private func showWindow(title: String, orderFront: Bool) {
    // The display under the pointer, where the picker and capsule are.
    let mouse = NSEvent.mouseLocation
    guard let screen = NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) })
      ?? NSScreen.main ?? NSScreen.screens.first else { return }
    let visible = screen.visibleFrame
    let height = (visible.height * 0.76).rounded()
    let width = (height / aspect).rounded()
    let frame = NSRect(
      x: (visible.midX - width / 2).rounded(),
      y: (visible.midY - height / 2 - 12).rounded(),
      width: width,
      height: height
    )
    let panel = MirrorPanel(
      contentRect: frame,
      styleMask: [.borderless, .resizable, .nonactivatingPanel],
      backing: .buffered,
      defer: false
    )
    panel.title = title
    panel.isMovableByWindowBackground = true
    panel.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue)
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = true
    panel.hidesOnDeactivate = false
    panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
    panel.contentAspectRatio = NSSize(width: 1, height: aspect)
    panel.delegate = self
    panel.onMarker = { emit(["event": "marker", "at": nowMs()]) }

    let view = PhoneCaseView(frame: NSRect(origin: .zero, size: frame.size))
    view.autoresizingMask = [.width, .height]
    panel.contentView = view
    view.needsLayout = true
    view.layoutSubtreeIfNeeded()
    window = panel
    caseView = view
    if orderFront {
      panel.orderFrontRegardless()
      emitRect()
    }
  }

  /// Shown again (the picker's iPhone mode) on another display than the pointer's: centred on
  /// the pointer's display, where the picker and capsule are.
  private func moveToPointerScreen() {
    guard let window else { return }
    let mouse = NSEvent.mouseLocation
    guard let screen = NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) }),
      !screen.frame.contains(NSPoint(x: window.frame.midX, y: window.frame.midY))
    else { return }
    let visible = screen.visibleFrame
    var frame = window.frame
    if frame.height > visible.height {
      frame.size = NSSize(width: (visible.height * 0.76 / aspect).rounded(), height: (visible.height * 0.76).rounded())
    }
    frame.origin = NSPoint(x: (visible.midX - frame.width / 2).rounded(), y: (visible.midY - frame.height / 2).rounded())
    window.setFrame(frame, display: true)
  }

  private func fitWindow() {
    guard let window else { return }
    var frame = window.frame
    let width = (frame.height / aspect).rounded()
    frame.origin.x = (frame.midX - width / 2).rounded()
    frame.size.width = width
    window.contentAspectRatio = NSSize(width: 1, height: aspect)
    window.setFrame(frame, display: true)
    caseView?.needsLayout = true
    caseView?.layoutSubtreeIfNeeded()
    if !hiddenPreview {
      emitRect()
    }
  }

  func windowDidMove(_: Notification) {
    if !hiddenPreview { emitRect() }
  }

  func windowDidResize(_: Notification) {
    caseView?.needsLayout = true
    caseView?.layoutSubtreeIfNeeded()
    if !hiddenPreview { emitRect() }
  }

  /// The picture's frame in global display coordinates with a top-left origin, the space
  /// `CGEventGetLocation` reports, so Scrawl can normalize pointer and pen positions into it.
  private func emitRect() {
    guard let window, let primary = NSScreen.screens.first else { return }
    let frame = window.frame
    emit(["event": "rect", "x": frame.minX, "y": primary.frame.maxY - frame.maxY, "width": frame.width, "height": frame.height])
  }

  // MARK: Commands

  private func readCommands() {
    Thread.detachNewThread {
      while let line = readLine() {
        let command = line.trimmingCharacters(in: .whitespaces)
        DispatchQueue.main.async {
          if command == "stop" {
            self.stop()
          } else if command == "cancel" {
            self.cancel()
          } else if command == "show" {
            self.hiddenPreview = false
            self.moveToPointerScreen()
            self.window?.orderFrontRegardless()
            self.emitRect()
          } else if command == "hide" {
            self.hiddenPreview = true
            self.window?.orderOut(nil)
          } else if command == "start" {
            self.startRecording(path: nil)
          } else if command.hasPrefix("start ") {
            let path = String(command.dropFirst(6)).trimmingCharacters(in: .whitespaces)
            self.startRecording(path: path)
          }
        }
      }
      DispatchQueue.main.async { self.stop() }
    }
  }

  private func startRecording(path: String?) {
    if let path, !path.isEmpty {
      output = URL(filePath: path)
    }
    previewOnly = false
    if hiddenPreview {
      hiddenPreview = false
      window?.orderFrontRegardless()
      emitRect()
    }
    startMovieWhenReady()
  }

  private func stop() {
    guard !stopping else { return }
    stopping = true
    window?.orderOut(nil)
    guard phoneInput != nil, !previewOnly, movieRequested else {
      finish(["event": "cancelled"])
      return
    }
    let movieOutput = movieOutput, session = session
    queue.async {
      if movieOutput.isRecording { movieOutput.stopRecording() } else { session.stopRunning() }
    }
    // If the file output never reports back, stopping the session finalizes the file.
    DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
      guard !self.finished else { return }
      self.queue.async { session.stopRunning() }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + 10) {
      guard !self.finished else { return }
      self.die("Recording did not finish")
    }
  }

  private func cancel() {
    cancelled = true
    stop()
    if phoneInput == nil { finish(["event": "cancelled"]) }
  }

  private func die(_ message: String) {
    finish(["event": "error", "message": message])
  }

  private func finish(_ event: [String: Any]) {
    guard !finished else { return }
    finished = true
    emit(event)
    let session = session
    queue.async {
      session.stopRunning()
      DispatchQueue.main.async { exit(0) }
    }
  }

  // MARK: AVCaptureFileOutputRecordingDelegate

  func fileOutput(_: AVCaptureFileOutput, didStartRecordingTo _: URL, from _: [AVCaptureConnection]) {
    let started = nowMs()
    DispatchQueue.main.async { emit(["event": "recording", "startedAt": started]) }
  }

  func fileOutput(_ fileOutput: AVCaptureFileOutput, didFinishRecordingTo url: URL, from _: [AVCaptureConnection], error: Error?) {
    let seconds = fileOutput.recordedDuration.seconds.isFinite ? fileOutput.recordedDuration.seconds : 0
    DispatchQueue.main.async {
      if self.cancelled {
        try? FileManager.default.removeItem(at: url)
        return self.finish(["event": "cancelled"])
      }
      guard Self.recordingSucceeded(error: error, stopping: self.stopping, seconds: seconds),
            FileManager.default.fileExists(atPath: url.path)
      else {
        return self.die("Recording failed: \(error?.localizedDescription ?? "no movie was written")")
      }
      self.finish(["event": "finished", "path": url.path, "durationMs": Int((seconds * 1000).rounded())])
    }
  }

  /// AVFoundation reports some clean stops as errors; only trust those it marks as finished,
  /// or a stop we asked for that left a real movie behind.
  static func recordingSucceeded(error: Error?, stopping: Bool, seconds: Double) -> Bool {
    guard let error else { return true }
    if (error as NSError).userInfo[AVErrorRecordingSuccessfullyFinishedKey] as? Bool == true { return true }
    return stopping && seconds > 0.1
  }
}

/// Key-capable so M can drop a marker while the mirror is frontmost.
final class MirrorPanel: NSPanel {
  var onMarker: (() -> Void)?
  override var canBecomeKey: Bool { true }
  override func keyDown(with event: NSEvent) {
    if event.charactersIgnoringModifiers?.lowercased() == "m", event.modifierFlags.intersection([.command, .control, .option]).isEmpty {
      onMarker?()
    } else {
      super.keyDown(with: event)
    }
  }
}

/// Phone-shaped bezel + screen container that explicitly lays out `AVCaptureVideoPreviewLayer`
/// on every resize/rotation so the live video layer is never 0x0.
final class PhoneCaseView: NSView {
  let bezelInset: CGFloat = 8
  let caseLayer = CALayer()
  let screenLayer = CALayer()
  var previewLayer: AVCaptureVideoPreviewLayer?
  let statusField: NSTextField

  override init(frame: NSRect) {
    statusField = NSTextField(labelWithString: "Connecting iPhone…")
    super.init(frame: frame)
    wantsLayer = true

    caseLayer.backgroundColor = NSColor(srgbRed: 0.11, green: 0.11, blue: 0.12, alpha: 0.98).cgColor
    caseLayer.borderColor = NSColor(calibratedWhite: 1.0, alpha: 0.24).cgColor
    caseLayer.borderWidth = 1.5
    caseLayer.cornerCurve = .continuous

    screenLayer.backgroundColor = NSColor.black.cgColor
    screenLayer.cornerCurve = .continuous
    screenLayer.masksToBounds = true
    caseLayer.addSublayer(screenLayer)

    statusField.font = NSFont.systemFont(ofSize: 13, weight: .medium)
    statusField.textColor = NSColor(calibratedWhite: 0.58, alpha: 1.0)
    statusField.alignment = .center
    statusField.drawsBackground = false
    statusField.isBezeled = false
    statusField.isEditable = false
    statusField.isSelectable = false
    addSubview(statusField)
  }

  @available(*, unavailable)
  required init?(coder _: NSCoder) { fatalError() }

  func attachSession(_ session: AVCaptureSession) {
    statusField.isHidden = true
    if previewLayer == nil {
      let layer = AVCaptureVideoPreviewLayer(session: session)
      layer.videoGravity = .resizeAspectFill
      layer.frame = screenLayer.bounds
      screenLayer.addSublayer(layer)
      previewLayer = layer
    }
    needsLayout = true
    layoutSubtreeIfNeeded()
  }

  override func makeBackingLayer() -> CALayer {
    caseLayer
  }

  override func layout() {
    super.layout()
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    let outerRadius = (min(bounds.width, bounds.height) * 0.135).rounded()
    caseLayer.cornerRadius = outerRadius
    let inner = bounds.insetBy(dx: bezelInset, dy: bezelInset)
    screenLayer.frame = inner
    screenLayer.cornerRadius = max(outerRadius - bezelInset, 24)
    previewLayer?.frame = screenLayer.bounds
    statusField.frame = NSRect(x: 16, y: bounds.midY - 10, width: max(bounds.width - 32, 0), height: 20)
    CATransaction.commit()
  }
}

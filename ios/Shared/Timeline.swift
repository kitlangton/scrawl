import Foundation

/// The `timeline.json` the Mac reads beside the movie (`phone::Timeline` in Scrawl): ScrawlKit's
/// keys, with `capture` saying how this app recorded it. Times are seconds from the movie's first
/// frame. There are no touches or strokes: iOS doesn't let one app see touches in another.
struct Timeline: Sendable {
  var recordedAt: String
  var duration: TimeInterval
  var capture: Capture.Kind
  /// The screen in points and its scale.
  var screen: (width: Double, height: Double, scale: Double)?
  /// Flagged moments (`⚑`, each gets a keyframe).
  var markers: [Double] = []
  var events: [(t: Double, label: String)] = []
  /// Named stretches (`⟨Screenshot 2 of 3⟩` in the summary).
  var screens: [(t: Double, name: String)] = []
  var audio = false
  /// The session it's addressed to; none lets the Mac pick.
  var session: String?
  var note: String?

  func json() throws -> Data {
    var info: [String: Any] = [
      "recordedAt": recordedAt,
      "duration": Self.round(duration),
      "capture": capture.rawValue,
      "device": Self.deviceName,
      "system": Self.system,
      "markers": markers.map(Self.round),
      "events": events.map { ["t": Self.round($0.t), "label": $0.label] },
      "touches": [Any](),
      "strokes": [Any](),
      "audio": audio,
    ]
    if let screen {
      info["screen"] = ["width": screen.width, "height": screen.height, "scale": screen.scale]
    }
    if !screens.isEmpty {
      info["screens"] = screens.map { ["t": Self.round($0.t), "name": $0.name] }
    }
    if let session, !session.isEmpty { info["session"] = session }
    if let note = note?.trimmingCharacters(in: .whitespacesAndNewlines), !note.isEmpty { info["note"] = note }
    return try JSONSerialization.data(withJSONObject: info, options: [.prettyPrinted, .sortedKeys])
  }

  /// "iPhone" or "iPad", as ScrawlKit's timelines say (iOS doesn't give apps the owner's name for
  /// the device). Read without UIKit, which wants the main thread.
  static let deviceName: String = {
    var info = utsname()
    uname(&info)
    let machine = withUnsafeBytes(of: &info.machine) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
    let model = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] ?? machine
    return model.hasPrefix("iPad") ? "iPad" : "iPhone"
  }()

  /// "iOS 27.0", as `UIDevice` spells it.
  static let system: String = {
    let version = ProcessInfo.processInfo.operatingSystemVersion
    let patch = version.patchVersion > 0 ? ".\(version.patchVersion)" : ""
    return "iOS \(version.majorVersion).\(version.minorVersion)\(patch)"
  }()

  static func round(_ value: Double) -> Double { (value * 1000).rounded() / 1000 }
}

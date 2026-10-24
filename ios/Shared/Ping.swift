import Foundation

/// A Darwin notification between the app and its extensions. Pings carry nothing: whatever
/// changed is on disk or in the shared defaults.
enum Ping: String, Sendable {
  /// A capture was created or changed.
  case capturesChanged = "captures-changed"
  /// Flag this moment in the recording (the Flag Moment shortcut).
  case flag

  var name: String { "\(AppGroup.identifier).\(rawValue)" }

  func post() {
    CFNotificationCenterPostNotification(CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name as CFString), nil, nil, true)
  }

  /// Calls `handler` (on an arbitrary thread) for each ping until the observer is released.
  func observe(_ handler: @escaping @Sendable () -> Void) -> PingObserver {
    PingObserver(name: name, handler: handler)
  }
}

final class PingObserver: Sendable {
  private let name: String
  private let handler: @Sendable () -> Void

  fileprivate init(name: String, handler: @escaping @Sendable () -> Void) {
    self.name = name
    self.handler = handler
    CFNotificationCenterAddObserver(
      CFNotificationCenterGetDarwinNotifyCenter(),
      Unmanaged.passUnretained(self).toOpaque(),
      { _, observer, _, _, _ in
        guard let observer else { return }
        Unmanaged<PingObserver>.fromOpaque(observer).takeUnretainedValue().handler()
      },
      name as CFString,
      nil,
      .deliverImmediately
    )
  }

  deinit {
    CFNotificationCenterRemoveObserver(
      CFNotificationCenterGetDarwinNotifyCenter(),
      Unmanaged.passUnretained(self).toOpaque(),
      CFNotificationName(name as CFString),
      nil
    )
  }
}

/// Which capture the broadcast extension is recording, if any, with a heartbeat so a recording
/// whose extension was killed doesn't look like it's still going.
enum ActiveBroadcast {
  private static let idKey = "activeBroadcast"
  private static let beatKey = "activeBroadcastBeat"
  /// The extension beats every few seconds; a beat older than this means it's gone.
  static let staleAfter: TimeInterval = 15

  static func begin(_ id: String) {
    AppGroup.defaults.set(id, forKey: idKey)
    beat()
  }

  static func beat(at date: Date = Date()) {
    AppGroup.defaults.set(date.timeIntervalSince1970, forKey: beatKey)
  }

  static func end() {
    AppGroup.defaults.removeObject(forKey: idKey)
    AppGroup.defaults.removeObject(forKey: beatKey)
  }

  /// The capture being recorded now.
  static func current(now: Date = Date()) -> String? {
    guard let id = AppGroup.defaults.string(forKey: idKey) else { return nil }
    let beat = AppGroup.defaults.double(forKey: beatKey)
    return now.timeIntervalSince1970 - beat < staleAfter ? id : nil
  }

  /// A capture marked as recording that nothing is recording any more: a broadcast whose
  /// extension stopped beating, or shared items that were still being made into a movie ten
  /// minutes on.
  static func isOrphaned(_ capture: Capture, now: Date = Date()) -> Bool {
    guard capture.state == .recording else { return false }
    let age = now.timeIntervalSince(capture.createdAt)
    switch capture.kind {
    case .broadcast: return current(now: now) != capture.id && age > staleAfter
    case .shared: return age > 10 * 60
    }
  }
}

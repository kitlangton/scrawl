import Foundation

/// What the Mac said became of an upload it took (ScrawlKit's `IngestReply`): it answers once
/// it has built the bundle and tried to queue it into the session,
/// `{"ok":true,"delivered":bool,"state":"sent"|"queued"|"processing"|"saved","sessionTitle":…}`.
/// Macs from before `delivered` answered as soon as the files arrived; that reads as delivered.
enum IngestReply: Codable, Equatable, Sendable {
  /// Queued into the OpenCode session `sessionTitle`.
  case delivered(sessionTitle: String)
  /// On the Mac, which couldn't reach the session and keeps trying on its own.
  case queued(sessionTitle: String)
  /// On the Mac, still being built or sent when it answered.
  case processing(sessionTitle: String)
  /// On the Mac and going no further (it has no session for it, or routing is off).
  case saved

  init(body: Data) {
    let json = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any] ?? [:]
    let title = (json["sessionTitle"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? "OpenCode"
    guard let delivered = json["delivered"] as? Bool, !delivered else {
      self = .delivered(sessionTitle: title)
      return
    }
    switch json["state"] as? String {
    case "queued": self = .queued(sessionTitle: title)
    case "processing": self = .processing(sessionTitle: title)
    default: self = .saved
    }
  }

  var isDelivered: Bool {
    if case .delivered = self { true } else { false }
  }
}

import Foundation

/// One recording on its way to the Mac: the whole screen recorded through the broadcast
/// extension, or what was shared into Scrawl (a screen recording, or screenshots made into a
/// short movie). The Mac builds the `.scrawl` bundle from its movie and timeline.
struct Capture: Codable, Equatable, Identifiable, Sendable {
  enum Kind: String, Codable, Sendable {
    case broadcast
    case shared
  }

  enum Failure: String, Codable, Sendable {
    /// 401/403: the Mac doesn't know this pairing anymore.
    case unauthorized
    /// 413, or over the size the Mac takes before sending.
    case tooLarge
    /// Another 4xx: the Mac turned these exact bytes down.
    case rejected
    /// The Mac couldn't build a bundle from it, more than once.
    case couldNotBuild
    /// The recording stopped before its movie was written (the extension was killed).
    case interrupted
  }

  enum State: Codable, Equatable, Sendable {
    case recording
    /// Kept on the phone until it's paired with a Mac.
    case notPaired
    case sending
    /// The Mac took it, and said what became of it.
    case accepted(IngestReply)
    /// Couldn't reach the Mac (or it answered with a transient error); tried again at
    /// `nextAttemptAt`, when the app comes to the front, or after pairing again.
    case waiting
    case failed(Failure)
  }

  let id: String
  let kind: Kind
  let createdAt: Date
  /// When it started, as the timeline says (`recordedAt`): the Mac dedupes uploads on it.
  var recordedAt: String
  var duration: TimeInterval?
  var destination: Destination?
  var note: String?
  /// How many screenshots were shared, when it's made of screenshots.
  var screenshots: Int?
  var state: State
  var attempts = 0
  var lastError: String?
  var nextAttemptAt: Date?
  /// The background `URLSession` the last attempt went out on, and when.
  var uploadSession: String?
  var uploadStartedAt: Date?
  var updatedAt: Date

  init(id: String, kind: Kind, createdAt: Date, destination: Destination?, note: String? = nil, state: State) {
    self.id = id
    self.kind = kind
    self.createdAt = createdAt
    // Milliseconds, so two captures in one second aren't taken for copies of one.
    recordedAt = createdAt.formatted(.iso8601.year().month().day().time(includingFractionalSeconds: true).timeZone(separator: .omitted))
    self.destination = destination
    self.note = note
    self.state = state
    updatedAt = createdAt
  }

  /// Files are gone from the phone once the Mac has them, apart from what the list shows.
  var isDone: Bool {
    switch state {
    case .accepted: true
    default: false
    }
  }

  var canRetry: Bool {
    switch state {
    case .waiting, .notPaired: true
    case let .failed(failure): failure != .interrupted && failure != .tooLarge
    default: false
    }
  }

  /// 15 s, 30 s, 1 min, … up to 30 min (as ScrawlKit's outbox).
  static func backoff(afterAttempts attempts: Int) -> TimeInterval {
    min(15 * pow(2, Double(max(attempts, 1) - 1)), 30 * 60)
  }
}

extension Capture {
  /// "0:42" for a recording, "3 screenshots" for shared ones.
  var lengthText: String {
    if let screenshots, screenshots > 0 {
      return screenshots == 1 ? "Screenshot" : "\(screenshots) screenshots"
    }
    guard let duration else { return kind == .broadcast ? "Screen recording" : "Recording" }
    let seconds = Int(duration.rounded())
    return String(format: "%d:%02d", seconds / 60, seconds % 60)
  }

  /// One line for the list and the notification title.
  var statusTitle: String {
    let target = destination?.title
    switch state {
    case .recording: return "Recording…"
    case .notPaired: return "Saved on your iPhone"
    case .sending: return target.map { "Sending to \($0)…" } ?? "Sending to your Mac…"
    case let .accepted(reply):
      switch reply {
      case let .delivered(title): return "Sent to \(title)"
      case .queued, .processing: return "On your Mac"
      case .saved: return "Saved on your Mac"
      }
    case .waiting: return "Couldn't reach your Mac"
    case let .failed(failure):
      switch failure {
      case .unauthorized: return "Not paired anymore"
      case .tooLarge: return "Too long to send"
      case .rejected: return "Your Mac turned it down"
      case .couldNotBuild: return "Your Mac couldn't build it"
      case .interrupted: return "Recording stopped early"
      }
    }
  }

  /// What happens next, for the list and the notification body.
  var statusDetail: String? {
    switch state {
    case .recording: return "Stop from the red indicator or Control Center."
    case .notPaired: return "Pair Scrawl with your Mac to send it."
    case .sending: return nil
    case let .accepted(reply):
      switch reply {
      case .delivered: return "Queued in the OpenCode session."
      case let .queued(title): return "Sends to \(title) when OpenCode answers."
      case let .processing(title): return "Sending to \(title)…"
      case .saved: return "Your Mac has no OpenCode session to send it to."
      }
    case .waiting: return "Scrawl sends it when your Mac is reachable."
    case let .failed(failure):
      switch failure {
      case .unauthorized: return "Run scrawl link on your Mac and scan the code again."
      case .tooLarge: return "Share it from Scrawl instead."
      case .rejected, .couldNotBuild: return lastError ?? "Share it from Scrawl instead."
      case .interrupted: return "iOS ended the recording before it was saved."
      }
    }
  }
}

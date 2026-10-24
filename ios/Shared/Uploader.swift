import Foundation

/// Sends captures to the Mac as file uploads on background `URLSession`s, so an upload the
/// broadcast or share extension starts just before it exits still finishes, and iOS wakes the app
/// to hear the Mac's reply (which comes once the bundle is built and queued into OpenCode).
///
/// iOS lets one process at a time use a background session, so each attempt gets its own
/// identifier (`<app group>.upload.<capture>.<attempt>`). When iOS wakes the app for one, the app
/// opens a session with that identifier and this same delegate hears how it ended.
///
/// An attempt owns its capture: sending claims it (`.sending`, under the store's lock, which every
/// process takes), and an outcome only counts while its attempt still owns it.
final class Uploader: NSObject, @unchecked Sendable {
  static let shared = Uploader(store: .shared)
  static let identifierPrefix = "\(AppGroup.identifier).upload."

  /// The Mac takes 250 MB at most, an HTTPS endpoint 95 MB.
  static func maxPayloadBytes(hosted: Bool) -> Int64 { hosted ? 95_000_000 : 250_000_000 }
  /// A `500` from the Mac (it couldn't build the bundle) is retried this many times in all.
  static let buildAttempts = 3
  /// An upload this young may still belong to the extension that started it, which iOS lets
  /// linger a little after it hands off: its session is left alone until then.
  static let handoffGrace: TimeInterval = 120

  let store: CaptureStore
  private let queue = DispatchQueue(label: "dev.kitlangton.scrawl.uploader")
  private var sessions: [String: URLSession] = [:]
  private var bodies: [String: Data] = [:]
  private var eventsDone: [String: @Sendable () -> Void] = [:]

  init(store: CaptureStore) {
    self.store = store
  }

  enum SendError: Error, LocalizedError {
    case missing
    /// Another attempt (in this process or another) is sending it, or it isn't waiting to be sent.
    case claimed
    case tooLarge

    var errorDescription: String? {
      switch self {
      case .missing: "The recording isn't on this iPhone anymore."
      case .claimed: "It's already on its way."
      case .tooLarge: "It's larger than your Mac takes."
      }
    }
  }

  /// Claims capture `id` and starts uploading it to the Scrawl at `base`, building its framed
  /// payload first. Only a capture that's waiting (or kept until pairing) can be claimed. Returns
  /// once the upload is handed to iOS; on failure the capture is given back, waiting.
  func send(_ id: String, to base: URL, pairing: Pairing) throws {
    guard store.load(id) != nil else { throw SendError.missing }
    guard let identifier = claim(id) else { throw SendError.claimed }
    do {
      let payload = store.payload(id)
      if !FileManager.default.fileExists(atPath: payload.path) {
        guard let timeline = try? Data(contentsOf: store.timeline(id)),
              FileManager.default.fileExists(atPath: store.movie(id).path)
        else { throw SendError.missing }
        try Self.frame(timeline: timeline, movie: store.movie(id), into: payload)
      }
      let bytes = (try? FileManager.default.attributesOfItem(atPath: payload.path)[.size] as? NSNumber)?.int64Value ?? 0
      guard bytes <= Self.maxPayloadBytes(hosted: base.scheme?.lowercased() == "https") else {
        finish(id, attempt: identifier, outcome: .failed(.tooLarge, nil))
        throw SendError.tooLarge
      }
      var request = URLRequest(url: base.appending(path: "ingest"))
      request.httpMethod = "POST"
      request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
      request.setValue(String(bytes), forHTTPHeaderField: "Content-Length")
      // The Mac answers once the bundle is built and queued into the session, or after this long.
      request.setValue("45", forHTTPHeaderField: "X-Scrawl-Wait")
      MacClient.authorize(&request, token: pairing.token)
      let task = session(identifier).uploadTask(with: request, fromFile: payload)
      task.taskDescription = id
      task.resume()
      log.info("upload \(id, privacy: .public) → \(base.absoluteString, privacy: .public) (\(bytes) bytes, \(identifier, privacy: .public))")
    } catch SendError.tooLarge {
      throw SendError.tooLarge
    } catch SendError.missing {
      finish(id, attempt: identifier, outcome: .failed(.interrupted, SendError.missing.localizedDescription))
      throw SendError.missing
    } catch {
      finish(id, attempt: identifier, outcome: .retry(error.localizedDescription))
      throw error
    }
  }

  /// Makes the next attempt the owner of capture `id` (`.sending`) and returns its session
  /// identifier, if the capture is waiting or kept until pairing; `nil` if it isn't (another
  /// attempt has it, or it's done).
  func claim(_ id: String, now: Date = Date()) -> String? {
    var identifier: String?
    store.transact(id) { capture in
      guard capture.state == .waiting || capture.state == .notPaired else { return false }
      capture.attempts += 1
      identifier = "\(Self.identifierPrefix)\(id).\(capture.attempts)"
      capture.state = .sending
      capture.nextAttemptAt = nil
      capture.uploadSession = identifier
      capture.uploadStartedAt = now
      return true
    }
    return identifier
  }

  /// iOS woke the app for a session's events: reopen it so they arrive here. `false` for
  /// sessions that aren't Scrawl's.
  func handleEvents(for identifier: String, completion: @escaping @Sendable () -> Void) -> Bool {
    guard identifier.hasPrefix(Self.identifierPrefix) else { return false }
    queue.sync {
      eventsDone[identifier] = completion
      _ = sessionOnQueue(identifier)
    }
    return true
  }

  /// Squares captures marked as sending with the uploads iOS actually has (at launch, and when
  /// the app comes to the front): reopening an attempt's session delivers a reply that came while
  /// the app wasn't running, a task still going (however long) is left to finish, and an attempt
  /// with no task left (iOS dropped it, or the app was quit from the switcher while it ran) gives
  /// the capture back, waiting. Uploads young enough to still be an extension's are skipped.
  func reconcile(now: Date = Date()) {
    for capture in store.all() where capture.state == .sending {
      guard let identifier = capture.uploadSession else {
        finish(capture.id, attempt: nil, outcome: .retry("The upload stopped before your Mac answered."))
        continue
      }
      guard now.timeIntervalSince(capture.uploadStartedAt ?? .distantPast) > Self.handoffGrace else { continue }
      session(identifier).getAllTasks { [self] tasks in
        guard tasks.isEmpty else { return }
        // A finished task's events may still be on their way; give them a moment.
        queue.asyncAfter(deadline: .now() + 3) { [self] in
          finish(capture.id, attempt: identifier, outcome: .retry("The upload stopped before your Mac answered."))
        }
      }
    }
  }

  // MARK: - Outcomes

  enum Outcome: Equatable {
    case accepted(IngestReply)
    case retry(String)
    case failed(Capture.Failure, String?)
  }

  /// What became of an attempt that ended with `status` (none when no response came), `body`
  /// and `error`, after `attempts` attempts.
  static func outcome(status: Int?, body: Data, error: (any Error)?, attempts: Int) -> Outcome {
    if let error {
      return .retry((error as NSError).localizedDescription)
    }
    guard let status else { return .retry("Your Mac didn't answer.") }
    let message = ((try? JSONSerialization.jsonObject(with: body)) as? [String: Any])?["error"] as? String
    switch status {
    case 200 ..< 300: return .accepted(IngestReply(body: body))
    case 401, 403: return .failed(.unauthorized, nil)
    case 413: return .failed(.tooLarge, nil)
    case 408, 429: return .retry("Your Mac was busy (\(status)).")
    case 400 ..< 500: return .failed(.rejected, message ?? "HTTP \(status)")
    case 500:
      // The Mac forgets a recording it couldn't build, so it can be sent again.
      return attempts < buildAttempts
        ? .retry(message ?? "Your Mac couldn't build it.")
        : .failed(.couldNotBuild, message)
    default: return .retry(message ?? "Your Mac answered \(status).")
    }
  }

  /// Records how attempt `attempt` of capture `id` ended, if that attempt still owns it (`nil`
  /// for a capture marked as sending without one). Returns the capture when it did.
  @discardableResult
  func finish(_ id: String, attempt: String?, outcome: Outcome) -> Capture? {
    var applied = false
    let capture = store.transact(id) { capture in
      guard capture.state == .sending, capture.uploadSession == attempt else { return false }
      switch outcome {
      case let .accepted(reply):
        capture.state = .accepted(reply)
        capture.lastError = nil
        capture.nextAttemptAt = nil
      case let .retry(message):
        capture.state = .waiting
        capture.lastError = message
        capture.nextAttemptAt = Date().addingTimeInterval(Capture.backoff(afterAttempts: capture.attempts))
      case let .failed(failure, message):
        capture.state = .failed(failure)
        capture.lastError = message
        capture.nextAttemptAt = nil
      }
      applied = true
      return true
    }
    guard applied, let capture else { return nil }
    log.info("upload \(id, privacy: .public) ended: \(String(describing: outcome), privacy: .public)")
    switch outcome {
    case .accepted:
      try? FileManager.default.removeItem(at: store.payload(id))
      Notifier.post(capture)
    case .retry:
      // Said once; later retries stay quiet. The app schedules its background retry.
      if capture.attempts <= 1 { Notifier.post(capture) }
      Retry.schedule()
    case .failed:
      Notifier.post(capture)
    }
    return capture
  }

  // MARK: - Framing

  /// The ingest payload: the timeline's length (4 bytes, big-endian), the timeline, the movie.
  /// Written to a file, streaming the movie, so it's never all in memory.
  static func frame(timeline: Data, movie: URL, into payload: URL) throws {
    let partial = payload.deletingLastPathComponent().appending(path: ".payload-\(UUID().uuidString).partial")
    guard FileManager.default.createFile(atPath: partial.path, contents: nil) else {
      throw CocoaError(.fileWriteUnknown)
    }
    do {
      let output = try FileHandle(forWritingTo: partial)
      defer { try? output.close() }
      var length = UInt32(timeline.count).bigEndian
      try output.write(contentsOf: Data(bytes: &length, count: 4))
      try output.write(contentsOf: timeline)
      let input = try FileHandle(forReadingFrom: movie)
      defer { try? input.close() }
      while let chunk = try input.read(upToCount: 1 << 20), !chunk.isEmpty {
        try output.write(contentsOf: chunk)
      }
      try output.synchronize()
    } catch {
      try? FileManager.default.removeItem(at: partial)
      throw error
    }
    try? FileManager.default.removeItem(at: payload)
    try FileManager.default.moveItem(at: partial, to: payload)
  }

  // MARK: - Sessions

  private func session(_ identifier: String) -> URLSession {
    queue.sync { sessionOnQueue(identifier) }
  }

  private func sessionOnQueue(_ identifier: String) -> URLSession {
    if let session = sessions[identifier] { return session }
    let configuration = URLSessionConfiguration.background(withIdentifier: identifier)
    configuration.sharedContainerIdentifier = AppGroup.identifier
    configuration.isDiscretionary = false
    configuration.sessionSendsLaunchEvents = true
    // Building the bundle takes the Mac a while after the bytes land.
    configuration.timeoutIntervalForRequest = 120
    configuration.timeoutIntervalForResource = 24 * 60 * 60
    let delegateQueue = OperationQueue()
    delegateQueue.maxConcurrentOperationCount = 1
    delegateQueue.underlyingQueue = queue
    let session = URLSession(configuration: configuration, delegate: self, delegateQueue: delegateQueue)
    sessions[identifier] = session
    return session
  }

  private func bodyKey(_ session: URLSession, _ task: URLSessionTask) -> String {
    "\(session.configuration.identifier ?? "")#\(task.taskIdentifier)"
  }
}

extension Uploader: URLSessionDataDelegate {
  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    let key = bodyKey(session, dataTask)
    var body = bodies[key] ?? Data()
    guard body.count < 64 * 1024 else { return }
    body.append(data)
    bodies[key] = body
  }

  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
    let body = bodies.removeValue(forKey: bodyKey(session, task)) ?? Data()
    guard let id = task.taskDescription, let identifier = session.configuration.identifier else { return }
    let status = error == nil ? (task.response as? HTTPURLResponse)?.statusCode : nil
    let attempts = store.load(id)?.attempts ?? 1
    finish(id, attempt: identifier, outcome: Self.outcome(status: status, body: body, error: error, attempts: attempts))
    // One task per session: done with it, unless iOS is still delivering its events (then it's
    // let go once they're all in).
    if eventsDone[identifier] == nil {
      session.finishTasksAndInvalidate()
      sessions[identifier] = nil
    }
  }

  func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
    guard let identifier = session.configuration.identifier else { return }
    let done = eventsDone.removeValue(forKey: identifier)
    session.finishTasksAndInvalidate()
    sessions[identifier] = nil
    if let done { DispatchQueue.main.async { done() } }
  }
}

/// Asks for a background retry when a capture is left waiting. The app sets it (background
/// refresh is the app's alone); in the extensions it does nothing.
enum Retry {
  nonisolated(unsafe) static var schedule: @Sendable () -> Void = {}
}

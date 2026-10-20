import Foundation

/// Durable delivery for recordings sent to an HTTPS endpoint (a hosted Problems board rather than
/// a Mac on the LAN).
///
/// Each recording is written to disk as its framed ingest payload (4-byte big-endian timeline
/// length, timeline JSON, MP4) plus a small metadata file before anything is sent, and only
/// deleted after a 2xx. Uploads are file uploads on a background `URLSession`, so they continue
/// while the app is suspended or killed and never hold the movie in memory.
///
/// - 401 and 403 (the link was revoked), 413 (too large) and every other 4xx except 408 and 429
///   (the server rejected these exact bytes, so resending can't help) are permanent: the entry stays
///   on disk, marked failed, until the link is renewed or it's shared some other way.
/// - Everything else (no network, timeouts, 408, 429, 5xx) is retried with backoff, and
///   whenever `kick(force: true)` is called (network back, app foregrounded, launch).
/// - An entry is never uploading twice at once: in-flight tasks are tracked by entry ID, including
///   tasks a background session carried over from a previous launch.
///
/// The server dedupes on person and `recordedAt`, so sending an entry again after a lost 2xx is
/// safe. All state lives on `queue`; the session's delegate callbacks run there too.
final class Outbox: NSObject, @unchecked Sendable {
  /// The ingest server answers 413 above 95 MB.
  static let defaultMaxPayloadBytes: Int64 = 95_000_000

  enum Failure: String, Codable, Sendable {
    /// 401/403: the Scrawl link was revoked or is wrong.
    case unauthorized
    /// 413, or the payload is over the size limit before sending.
    case tooLarge
    /// Any other 4xx except 408/429 (malformed frame or timeline, missing Content-Length, …).
    case rejected
  }

  struct Entry: Codable, Sendable, Equatable {
    let id: String
    let target: URL
    let recordedAt: String
    let createdAt: Date
    let payloadBytes: Int64
    var attempts = 0
    var lastError: String?
    var failure: Failure?
    var nextAttemptAt: Date?
  }

  enum Event: Sendable, Equatable {
    /// An upload attempt began (the task may still wait for a network).
    case started(id: String)
    /// Bytes went out: `fraction` of the payload so far. Sent when the whole percent changes, or
    /// every `progressInterval` while bytes keep moving more slowly than that.
    case progress(id: String, fraction: Double)
    /// The server took it (a 2xx), and said what became of it.
    case sent(id: String, reply: IngestReply)
    /// A transient failure; the entry is tried again at `next` (or sooner on a kick).
    case retrying(id: String, error: String, next: Date)
    case failed(id: String, Failure)

    var id: String {
      switch self {
      case let .started(id), let .progress(id, _), let .sent(id, _), let .retrying(id, _, _), let .failed(id, _): id
      }
    }
  }

  static let progressInterval: TimeInterval = 5

  /// Whether bytes sent up to `percent` are worth an event after `last` was reported: a new whole
  /// percent, or a heartbeat every `progressInterval` while it creeps.
  static func reportsProgress(percent: Int, after last: (percent: Int, at: Date)?, now: Date) -> Bool {
    guard let last else { return true }
    return percent != last.percent || now.timeIntervalSince(last.at) >= progressInterval
  }

  enum Outcome: Equatable {
    case success
    case permanent(Failure)
    case retry
  }

  /// What to do after an upload attempt finished with `status` (nil when no response arrived).
  static func classify(status: Int?) -> Outcome {
    guard let status else { return .retry }
    switch status {
    case 200 ..< 300: return .success
    case 401, 403: return .permanent(.unauthorized)
    case 413: return .permanent(.tooLarge)
    case 408, 429: return .retry
    case 400 ..< 500: return .permanent(.rejected)
    default: return .retry
    }
  }

  /// 15 s, 30 s, 1 min, … up to 30 min.
  static func backoff(afterAttempts attempts: Int) -> TimeInterval {
    min(15 * pow(2, Double(max(attempts, 1) - 1)), 30 * 60)
  }

  let directory: URL
  let maxPayloadBytes: Int64
  private let queue = DispatchQueue(label: "dev.kitlangton.scrawl.outbox")
  private let onEvent: @Sendable (Event) -> Void
  private let now: @Sendable () -> Date
  private var session: URLSession!
  private var token: String?
  private var target: URL?
  /// Entry ID → task identifier for uploads in progress.
  private var inFlight: [String: Int] = [:]
  private var responses: [Int: Data] = [:]
  /// Task identifier → the last progress reported for it.
  private var reported: [Int: (percent: Int, at: Date)] = [:]
  /// The session's surviving tasks have been collected; until then kicks wait.
  private var isReconnected = false
  private var pendingKick: Bool?
  private var retryTimer: DispatchSourceTimer?
  private var backgroundEventsDone: (@Sendable () -> Void)?
  /// After `invalidate`, nothing new starts (a task in an invalidated session would throw).
  private var isInvalidated = false

  init(
    directory: URL,
    configuration: URLSessionConfiguration,
    maxPayloadBytes: Int64 = Outbox.defaultMaxPayloadBytes,
    now: @escaping @Sendable () -> Date = { Date() },
    onEvent: @escaping @Sendable (Event) -> Void
  ) {
    self.directory = directory
    self.maxPayloadBytes = maxPayloadBytes
    self.now = now
    self.onEvent = onEvent
    super.init()
    let delegateQueue = OperationQueue()
    delegateQueue.maxConcurrentOperationCount = 1
    delegateQueue.underlyingQueue = queue
    session = URLSession(configuration: configuration, delegate: self, delegateQueue: delegateQueue)
    // A background session relaunched after a kill still owns the uploads it had: adopt them so
    // they aren't started a second time.
    session.getAllTasks { [self] tasks in
      queue.async { [self] in
        for task in tasks {
          if let id = task.taskDescription, task.state != .completed {
            inFlight[id] = task.taskIdentifier
          }
        }
        isReconnected = true
        if let force = pendingKick {
          pendingKick = nil
          kickOnQueue(force: force)
        }
      }
    }
  }

  /// The background session's configuration: uploads continue while the app is suspended or
  /// killed, and the system relaunches the app to report them.
  static func backgroundConfiguration(identifier: String) -> URLSessionConfiguration {
    let configuration = URLSessionConfiguration.background(withIdentifier: identifier)
    configuration.isDiscretionary = false
    configuration.sessionSendsLaunchEvents = true
    configuration.timeoutIntervalForResource = 24 * 60 * 60
    return configuration
  }

  /// Where uploads go and the bearer token (from the current pairing). Entries are sent to the
  /// original target only. Renewing a token may resume uploads to that same destination;
  /// changing destinations must never reroute previously captured data.
  func setCredentials(target: URL?, token: String?) {
    queue.async { [self] in
      self.target = target
      self.token = token
    }
  }

  /// Writes the framed payload and its metadata, protected and excluded from backup, and returns
  /// the entry. Nothing is sent until `kick`. With `failure`, the entry starts out failed (the link
  /// is known to be revoked) and is kept for when it's renewed.
  func enqueue(timeline: Data, movie: URL, target: URL, recordedAt: String, failure: Failure? = nil) async throws -> Entry {
    try await withCheckedThrowingContinuation { continuation in
      queue.async { [self] in
        continuation.resume(with: Result {
          try write(timeline: timeline, movie: movie, target: target, recordedAt: recordedAt, failure: failure)
        })
      }
    }
  }

  /// Starts uploads for entries that are due (all pending entries when `force`), skipping any
  /// already in flight or permanently failed.
  func kick(force: Bool) {
    queue.async { [self] in kickOnQueue(force: force) }
  }

  /// Clears permanent failures (after the link is renewed) and tries those entries again.
  func retryFailed() {
    queue.async { [self] in
      for var entry in loadEntries() where entry.failure != nil {
        entry.failure = nil
        entry.lastError = nil
        entry.nextAttemptAt = nil
        if entry.payloadBytes > maxPayloadBytes { entry.failure = .tooLarge }
        save(entry)
      }
      kickOnQueue(force: true)
    }
  }

  func entries() async -> [Entry] {
    await withCheckedContinuation { continuation in
      queue.async { [self] in continuation.resume(returning: loadEntries()) }
    }
  }

  /// Removes an entry and its payload (cancelling its upload if one is running).
  func remove(id: String) {
    queue.async { [self] in
      if let taskID = inFlight.removeValue(forKey: id) {
        session.getAllTasks { tasks in tasks.first { $0.taskIdentifier == taskID }?.cancel() }
      }
      try? FileManager.default.removeItem(at: payloadURL(id))
      try? FileManager.default.removeItem(at: metadataURL(id))
    }
  }

  /// Copies an entry's timeline and movie out of its payload into `folder` (for the share sheet).
  func extract(id: String, into folder: URL) async throws -> (movie: URL, timeline: URL) {
    try await withCheckedThrowingContinuation { continuation in
      queue.async { [self] in
        continuation.resume(with: Result { try extractOnQueue(id: id, into: folder) })
      }
    }
  }

  /// The system relaunched the app for this session's events: `completion` runs once they've all
  /// been delivered.
  func handleBackgroundEvents(completion: @escaping @Sendable () -> Void) {
    queue.async { [self] in backgroundEventsDone = completion }
  }

  /// Stops all uploads and the session (the device was unpaired). Files stay on disk.
  func invalidate() {
    queue.async { [self] in
      isInvalidated = true
      retryTimer?.cancel()
      retryTimer = nil
      session.invalidateAndCancel()
    }
  }

  // MARK: - Files

  private func payloadURL(_ id: String) -> URL { directory.appending(path: "\(id).payload") }
  private func metadataURL(_ id: String) -> URL { directory.appending(path: "\(id).json") }

  private func prepareDirectory() throws {
    let manager = FileManager.default
    if !manager.fileExists(atPath: directory.path) {
      try manager.createDirectory(at: directory, withIntermediateDirectories: true, attributes: Self.protection)
    }
    var directory = directory
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try directory.setResourceValues(values)
  }

  /// Readable after the first unlock, so background uploads work while the phone is locked.
  private static var protection: [FileAttributeKey: Any]? {
    #if os(iOS)
      [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
    #else
      nil
    #endif
  }

  private func write(timeline: Data, movie: URL, target: URL, recordedAt: String, failure: Failure?) throws -> Entry {
    try prepareDirectory()
    let stamp = recordedAt.filter { $0.isNumber }
    let id = "\(stamp.isEmpty ? "recording" : stamp)-\(UUID().uuidString.prefix(8).lowercased())"
    let payload = payloadURL(id)
    guard FileManager.default.createFile(atPath: payload.path, contents: nil, attributes: Self.protection) else {
      throw CocoaError(.fileWriteUnknown)
    }
    do {
      let output = try FileHandle(forWritingTo: payload)
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
      try? FileManager.default.removeItem(at: payload)
      throw error
    }
    let bytes = (try? FileManager.default.attributesOfItem(atPath: payload.path)[.size] as? NSNumber)?.int64Value ?? 0
    var entry = Entry(id: id, target: target, recordedAt: recordedAt, createdAt: now(), payloadBytes: bytes)
    entry.failure = bytes > maxPayloadBytes ? .tooLarge : failure
    save(entry)
    guard FileManager.default.fileExists(atPath: metadataURL(id).path) else {
      try? FileManager.default.removeItem(at: payload)
      throw CocoaError(.fileWriteUnknown)
    }
    return entry
  }

  private func save(_ entry: Entry) {
    guard let data = try? JSONEncoder().encode(entry) else { return }
    #if os(iOS)
      try? data.write(to: metadataURL(entry.id), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    #else
      try? data.write(to: metadataURL(entry.id), options: .atomic)
    #endif
  }

  private func load(_ id: String) -> Entry? {
    guard let data = try? Data(contentsOf: metadataURL(id)) else { return nil }
    return try? JSONDecoder().decode(Entry.self, from: data)
  }

  /// Entries whose payload is on disk, oldest first.
  private func loadEntries() -> [Entry] {
    let names = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
    return names
      .filter { $0.hasSuffix(".json") }
      .compactMap { load(String($0.dropLast(5))) }
      .filter { FileManager.default.fileExists(atPath: payloadURL($0.id).path) }
      .sorted { $0.createdAt < $1.createdAt }
  }

  private func extractOnQueue(id: String, into folder: URL) throws -> (movie: URL, timeline: URL) {
    let input = try FileHandle(forReadingFrom: payloadURL(id))
    defer { try? input.close() }
    guard let header = try input.read(upToCount: 4), header.count == 4 else { throw CocoaError(.fileReadCorruptFile) }
    let length = header.reduce(0) { $0 << 8 | Int($1) }
    guard let timeline = try input.read(upToCount: length), timeline.count == length else { throw CocoaError(.fileReadCorruptFile) }
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    let timelineURL = folder.appending(path: "\(id).json")
    let movieURL = folder.appending(path: "\(id).mp4")
    try timeline.write(to: timelineURL)
    FileManager.default.createFile(atPath: movieURL.path, contents: nil, attributes: Self.protection)
    let output = try FileHandle(forWritingTo: movieURL)
    defer { try? output.close() }
    while let chunk = try input.read(upToCount: 1 << 20), !chunk.isEmpty {
      try output.write(contentsOf: chunk)
    }
    return (movieURL, timelineURL)
  }

  // MARK: - Sending

  private func kickOnQueue(force: Bool) {
    guard !isInvalidated else { return }
    guard isReconnected else {
      pendingKick = (pendingKick ?? false) || force
      return
    }
    let now = now()
    var nextDue: Date?
    for entry in loadEntries() where entry.failure == nil && inFlight[entry.id] == nil {
      if force || (entry.nextAttemptAt ?? .distantPast) <= now {
        start(entry)
      } else if let due = entry.nextAttemptAt {
        nextDue = min(nextDue ?? due, due)
      }
    }
    scheduleRetry(at: nextDue)
  }

  private func start(_ entry: Entry) {
    guard let token, !token.isEmpty else { return }
    guard let base = target, base == entry.target else { return }
    var request = URLRequest(url: base.appending(path: "ingest"))
    request.httpMethod = "POST"
    request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
    request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
    // Apple reserves `Authorization` for the URL loading system, so a background session may drop
    // or replace it; the server also accepts the token here.
    request.setValue(token, forHTTPHeaderField: "X-Scrawl-Token")
    request.setValue(String(entry.payloadBytes), forHTTPHeaderField: "Content-Length")
    let task = session.uploadTask(with: request, fromFile: payloadURL(entry.id))
    task.taskDescription = entry.id
    inFlight[entry.id] = task.taskIdentifier
    var entry = entry
    entry.attempts += 1
    entry.nextAttemptAt = nil
    save(entry)
    task.resume()
    onEvent(.started(id: entry.id))
  }

  private func sentBodyData(_ task: URLSessionTask, sent: Int64, expected: Int64) {
    guard let id = task.taskDescription, inFlight[id] == task.taskIdentifier, expected > 0 else { return }
    let fraction = min(max(Double(sent) / Double(expected), 0), 1)
    let percent = Int(fraction * 100)
    let now = now()
    guard Self.reportsProgress(percent: percent, after: reported[task.taskIdentifier], now: now) else { return }
    reported[task.taskIdentifier] = (percent, now)
    onEvent(.progress(id: id, fraction: fraction))
  }

  private func scheduleRetry(at date: Date?) {
    retryTimer?.cancel()
    retryTimer = nil
    guard let date else { return }
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now() + max(date.timeIntervalSince(now()), 0.5))
    timer.setEventHandler { [weak self] in self?.kickOnQueue(force: false) }
    timer.resume()
    retryTimer = timer
  }

  private func finished(_ task: URLSessionTask, error: (any Error)?) {
    let body = responses.removeValue(forKey: task.taskIdentifier) ?? Data()
    reported[task.taskIdentifier] = nil
    guard let id = task.taskDescription else { return }
    if inFlight[id] == task.taskIdentifier { inFlight[id] = nil }
    // Removed while uploading (cancelled), already handled, or the session was torn down (the
    // entry stays as it was for the next pairing).
    guard !isInvalidated, var entry = load(id) else { return }
    let status = error == nil ? (task.response as? HTTPURLResponse)?.statusCode : nil
    switch Self.classify(status: status) {
    case .success:
      try? FileManager.default.removeItem(at: payloadURL(id))
      try? FileManager.default.removeItem(at: metadataURL(id))
      onEvent(.sent(id: id, reply: IngestReply(body: body)))
    case let .permanent(failure):
      entry.failure = failure
      entry.lastError = "HTTP \(status ?? 0)"
      save(entry)
      onEvent(.failed(id: id, failure))
    case .retry:
      let reason = error.map { ($0 as NSError).localizedDescription } ?? "HTTP \(status ?? 0)"
      let next = now().addingTimeInterval(Self.backoff(afterAttempts: entry.attempts))
      entry.lastError = reason
      entry.nextAttemptAt = next
      save(entry)
      onEvent(.retrying(id: id, error: reason, next: next))
      kickOnQueue(force: false)
    }
  }
}

extension Outbox: URLSessionDataDelegate {
  func urlSession(_: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    var body = responses[dataTask.taskIdentifier] ?? Data()
    guard body.count < 64 * 1024 else { return }
    body.append(data)
    responses[dataTask.taskIdentifier] = body
  }

  func urlSession(_: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
    finished(task, error: error)
  }

  func urlSession(
    _: URLSession,
    task: URLSessionTask,
    didSendBodyData _: Int64,
    totalBytesSent: Int64,
    totalBytesExpectedToSend: Int64
  ) {
    sentBodyData(task, sent: totalBytesSent, expected: totalBytesExpectedToSend)
  }

  func urlSessionDidFinishEvents(forBackgroundURLSession _: URLSession) {
    guard let done = backgroundEventsDone else { return }
    backgroundEventsDone = nil
    DispatchQueue.main.async { done() }
  }
}

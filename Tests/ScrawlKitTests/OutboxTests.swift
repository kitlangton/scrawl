import Foundation
@testable import ScrawlKit
import Testing

/// Stands in for the Problems board's ingest endpoint.
final class StubServer: URLProtocol, @unchecked Sendable {
  struct Reply {
    var status = 200
    var body = Data(#"{"ok":true,"sessionTitle":"Problems · Ledger","item":"I-1"}"#.utf8)
    /// Fail the connection instead of answering (after the server has read the upload).
    var error: URLError?
    var delay: TimeInterval = 0
  }

  struct Received {
    let request: URLRequest
    let body: Data
    /// Files in the outbox directory when the request arrived.
    let filesOnDisk: [String]
  }

  private static let lock = NSLock()
  nonisolated(unsafe) private static var replies: [Reply] = []
  nonisolated(unsafe) private static var log: [Received] = []
  nonisolated(unsafe) static var watchedDirectory: URL?

  static func reset(_ replies: [Reply] = []) {
    lock.withLock {
      self.replies = replies
      log = []
    }
  }

  static func enqueue(_ reply: Reply) { lock.withLock { replies.append(reply) } }
  static var received: [Received] { lock.withLock { log } }

  override class func canInit(with _: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

  override func startLoading() {
    let body = Self.readBody(request)
    let files = Self.watchedDirectory.flatMap { try? FileManager.default.contentsOfDirectory(atPath: $0.path) } ?? []
    let reply: Reply = Self.lock.withLock {
      Self.log.append(Received(request: request, body: body, filesOnDisk: files.sorted()))
      return Self.replies.isEmpty ? Reply() : Self.replies.removeFirst()
    }
    DispatchQueue.global().asyncAfter(deadline: .now() + reply.delay) { [self] in
      if let error = reply.error {
        client?.urlProtocol(self, didFailWithError: error)
        return
      }
      let response = HTTPURLResponse(url: request.url!, statusCode: reply.status, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json"])!
      client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
      client?.urlProtocol(self, didLoad: reply.body)
      client?.urlProtocolDidFinishLoading(self)
    }
  }

  override func stopLoading() {}

  private static func readBody(_ request: URLRequest) -> Data {
    if let body = request.httpBody { return body }
    guard let stream = request.httpBodyStream else { return Data() }
    stream.open()
    defer { stream.close() }
    var data = Data()
    var buffer = [UInt8](repeating: 0, count: 64 * 1024)
    while stream.hasBytesAvailable {
      let count = stream.read(&buffer, maxLength: buffer.count)
      if count <= 0 { break }
      data.append(buffer, count: count)
    }
    return data
  }
}

/// Collects the outbox's events.
final class EventLog: @unchecked Sendable {
  private let lock = NSLock()
  private var events: [Outbox.Event] = []
  /// Also follows the events, the way `Scrawl` drives the strip.
  let delivery = DeliveryProbe()
  func append(_ event: Outbox.Event) {
    lock.withLock { events.append(event) }
    delivery.apply(event)
  }

  var all: [Outbox.Event] { lock.withLock { events } }
}

/// Feeds a `DeliveryTracker` the outbox's events and a network path, keeping every change in
/// what the strip would say.
final class DeliveryProbe: @unchecked Sendable {
  private let lock = NSLock()
  private var tracker: DeliveryTracker?
  private var pathSatisfied = true
  private var log: [DeliveryTracker.Display] = []

  func track(_ entry: String) {
    lock.withLock {
      tracker = DeliveryTracker(entry: entry, pathSatisfied: pathSatisfied, now: Date())
      note()
    }
  }

  func apply(_ event: Outbox.Event) {
    lock.withLock {
      tracker?.apply(event, at: Date())
      note()
    }
  }

  func setPath(satisfied: Bool) {
    lock.withLock {
      pathSatisfied = satisfied
      tracker?.setPath(satisfied: satisfied, at: Date())
      note()
    }
  }

  /// What the strip says now (also logged if it changed with time alone).
  var current: DeliveryTracker.Display? {
    lock.withLock {
      note()
      return log.last
    }
  }

  var displays: [DeliveryTracker.Display] { lock.withLock { log } }

  /// The states in order, ignoring progress within "Sending…".
  var states: [String] {
    displays.map { display in
      switch display {
      case .sending: "sending"
      case .stillSending: "stillSending"
      case .queued: "queued"
      case .sent: "sent"
      case .failed: "failed"
      }
    }
    .reduce(into: []) { states, state in
      if states.last != state { states.append(state) }
    }
  }

  private func note() {
    guard let display = tracker?.display(at: Date()), log.last != display else { return }
    log.append(display)
  }
}

/// Polls until `condition` holds or `timeout` passes.
func eventually(_ timeout: TimeInterval = 5, _ condition: () async -> Bool) async -> Bool {
  let deadline = Date().addingTimeInterval(timeout)
  while Date() < deadline {
    if await condition() { return true }
    try? await Task.sleep(for: .milliseconds(20))
  }
  return await condition()
}

@Suite(.serialized)
struct OutboxTests {
  let directory: URL
  let movie: URL
  let events = EventLog()
  let target = URL(string: "https://problems.example.com/api/scrawl/ledger")!
  let timeline = Data(#"{"recordedAt":"2026-10-14T18:00:00Z","app":"Ledger"}"#.utf8)
  let movieBytes = Data((0 ..< 300_000).map { UInt8($0 % 251) })

  init() throws {
    let root = FileManager.default.temporaryDirectory.appending(path: "scrawl-outbox-tests-\(UUID().uuidString)")
    directory = root.appending(path: "ScrawlOutbox")
    movie = root.appending(path: "movie.mp4")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    try movieBytes.write(to: movie)
    StubServer.reset()
    StubServer.watchedDirectory = directory
  }

  func makeOutbox(maxPayloadBytes: Int64 = Outbox.defaultMaxPayloadBytes) -> Outbox {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [StubServer.self]
    let events = events
    let outbox = Outbox(directory: directory, configuration: configuration, maxPayloadBytes: maxPayloadBytes) { events.append($0) }
    outbox.setCredentials(target: target, token: "pbcap_test")
    return outbox
  }

  var expectedPayload: Data {
    var length = UInt32(timeline.count).bigEndian
    return Data(bytes: &length, count: 4) + timeline + movieBytes
  }

  var filesOnDisk: [String] {
    ((try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []).sorted()
  }

  @Test func changingThePairingCannotRerouteQueuedRecordings() async throws {
    let outbox = makeOutbox()
    defer { outbox.invalidate() }
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    let other = URL(string: "https://other.example.com/upload")!
    outbox.setCredentials(target: other, token: "other-token")
    let newEntry = try await outbox.enqueue(timeline: timeline, movie: movie, target: other, recordedAt: "2026-10-14T19:00:00Z")
    outbox.kick(force: true)
    // A new-destination upload proves reconnection and dispatch have completed, while the
    // old recording must never even have started an upload with the new credentials.
    #expect(await eventually { events.all.contains(.sent(id: newEntry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(!events.all.contains(.started(id: entry.id)))
    let entries = await outbox.entries()
    #expect(entries.contains { $0.id == entry.id })
    outbox.setCredentials(target: target, token: "renewed-token")
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(StubServer.received.count == 2)
    #expect(StubServer.received.last?.request.url == target.appending(path: "ingest"))
  }

  @Test func persistsTheFramedPayloadBeforeSendingAndDeletesItAfter2xx() async throws {
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    #expect(filesOnDisk == ["\(entry.id).json", "\(entry.id).payload"])
    #expect(try Data(contentsOf: directory.appending(path: "\(entry.id).payload")) == expectedPayload)
    #expect(entry.payloadBytes == Int64(expectedPayload.count))
    #expect(try directory.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
    #expect(StubServer.received.isEmpty)

    outbox.kick(force: false)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    let received = try #require(StubServer.received.first)
    #expect(received.request.url == target.appending(path: "ingest"))
    #expect(received.request.httpMethod == "POST")
    #expect(received.request.value(forHTTPHeaderField: "Authorization") == "Bearer pbcap_test")
    #expect(received.request.value(forHTTPHeaderField: "X-Scrawl-Token") == "pbcap_test")
    #expect(received.request.value(forHTTPHeaderField: "Content-Type") == "application/octet-stream")
    #expect(received.request.value(forHTTPHeaderField: "Content-Length") == String(expectedPayload.count))
    // The payload was on disk while it was being sent.
    #expect(received.filesOnDisk.contains("\(entry.id).payload"))
    #expect(received.body == expectedPayload)
    #expect(await eventually { filesOnDisk.isEmpty })
  }

  @Test func aMacThatKeptItButCouldNotForwardItIsNotAskedAgain() async throws {
    StubServer.reset([.init(body: Data(#"{"ok":true,"delivered":false,"state":"queued","sessionTitle":"UP DOWN"}"#.utf8))])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    outbox.kick(force: false)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .queued(sessionTitle: "UP DOWN"))) })
    #expect(await eventually { filesOnDisk.isEmpty })
    outbox.kick(force: true)
    try await Task.sleep(for: .milliseconds(200))
    #expect(StubServer.received.count == 1)
  }

  @Test(arguments: [500, 502, 503, 429, 408])
  func transientStatusesAreRetriedWithBackoffAndTheFileIsKept(status: Int) async throws {
    StubServer.reset([.init(status: status)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    outbox.kick(force: false)
    #expect(await eventually { events.all.contains { if case .retrying(entry.id, _, _) = $0 { true } else { false } } })
    let kept = try #require(await outbox.entries().first)
    #expect(kept.attempts == 1)
    #expect(kept.lastError == "HTTP \(status)")
    #expect(kept.failure == nil)
    #expect((kept.nextAttemptAt ?? .distantPast) > Date())
    #expect(filesOnDisk.count == 2)

    // Not due yet: an ordinary kick leaves it waiting out its backoff.
    outbox.kick(force: false)
    try await Task.sleep(for: .milliseconds(200))
    #expect(StubServer.received.count == 1)

    // Network back / foreground / launch: tried again now, and deleted on 2xx.
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(StubServer.received.count == 2)
    #expect(await eventually { filesOnDisk.isEmpty })
  }

  @Test func noNetworkIsRetried() async throws {
    StubServer.reset([.init(error: URLError(.notConnectedToInternet)), .init(error: URLError(.timedOut))])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    outbox.kick(force: false)
    #expect(await eventually { events.all.filter { if case .retrying = $0 { true } else { false } }.count == 1 })
    outbox.kick(force: true)
    #expect(await eventually { events.all.filter { if case .retrying = $0 { true } else { false } }.count == 2 })
    #expect(try #require(await outbox.entries().first).attempts == 2)
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(await eventually { filesOnDisk.isEmpty })
  }

  @Test(arguments: [(401, Outbox.Failure.unauthorized), (403, .unauthorized), (413, .tooLarge)])
  func permanentFailuresKeepTheFileAndStopRetrying(status: Int, failure: Outbox.Failure) async throws {
    StubServer.reset([.init(status: status, body: Data(#"{"error":"no"}"#.utf8))])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    outbox.kick(force: false)
    #expect(await eventually { events.all.contains(.failed(id: entry.id, failure)) })
    #expect(filesOnDisk == ["\(entry.id).json", "\(entry.id).payload"])
    #expect(try #require(await outbox.entries().first).failure == failure)

    outbox.kick(force: true)
    try await Task.sleep(for: .milliseconds(200))
    #expect(StubServer.received.count == 1)

    // A renewed link sends it after all.
    outbox.retryFailed()
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(await eventually { filesOnDisk.isEmpty })
  }

  @Test func aLostReplyIsSentAgainWithTheSameBytes() async throws {
    // The server stored the upload but the connection dropped before its 2xx arrived; the resend
    // is identical (same recordedAt), so the server's dedupe answers 200 with the same item.
    StubServer.reset([.init(error: URLError(.networkConnectionLost)), .init(status: 200)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    outbox.kick(force: false)
    #expect(await eventually { events.all.contains { if case .retrying = $0 { true } else { false } } })
    #expect(filesOnDisk.count == 2)
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    let bodies = StubServer.received.map(\.body)
    #expect(bodies.count == 2)
    #expect(bodies[0] == expectedPayload)
    #expect(bodies[1] == expectedPayload)
    #expect(await eventually { filesOnDisk.isEmpty })
  }

  @Test func anEntryIsNeverUploadingTwiceAtOnce() async throws {
    StubServer.reset([.init(delay: 0.6)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    for _ in 0 ..< 5 {
      outbox.kick(force: true)
    }
    try await Task.sleep(for: .milliseconds(250))
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    try await Task.sleep(for: .milliseconds(200))
    #expect(StubServer.received.count == 1)
  }

  @Test func payloadsOverTheLimitAreNeverSent() async throws {
    let outbox = makeOutbox(maxPayloadBytes: 100_000)
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    #expect(entry.failure == .tooLarge)
    outbox.kick(force: true)
    outbox.retryFailed()
    try await Task.sleep(for: .milliseconds(300))
    #expect(StubServer.received.isEmpty)
    #expect(filesOnDisk.count == 2)
  }

  @Test func aRevokedLinkQueuesTheEntryAsFailedWithoutSending() async throws {
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z", failure: .unauthorized)
    outbox.kick(force: true)
    try await Task.sleep(for: .milliseconds(200))
    #expect(StubServer.received.isEmpty)
    #expect(try #require(await outbox.entries().first).failure == .unauthorized)
    outbox.retryFailed()
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
  }

  @Test func entriesSurviveARestartAndAreSentBySeparateOutboxes() async throws {
    StubServer.reset([.init(status: 503)])
    let first = makeOutbox()
    let entry = try await first.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    first.kick(force: false)
    #expect(await eventually { events.all.contains { if case .retrying = $0 { true } else { false } } })
    first.invalidate()

    // A new launch picks the entry up from disk.
    let second = makeOutbox()
    #expect(await second.entries().map(\.id) == [entry.id])
    second.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(await eventually { filesOnDisk.isEmpty })
  }

  @Test func sharingExtractsTheOriginalTimelineAndMovie() async throws {
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    let folder = directory.deletingLastPathComponent().appending(path: "share")
    let files = try await outbox.extract(id: entry.id, into: folder)
    #expect(try Data(contentsOf: files.timeline) == timeline)
    #expect(try Data(contentsOf: files.movie) == movieBytes)
  }

  // MARK: - What the strip says

  @Test func aSlowUploadIsSendingUntilItLandsNeverQueued() async throws {
    // Ledger I-37: online, but the upload took longer than the old 4 s window.
    StubServer.reset([.init(delay: 5)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    events.delivery.track(entry.id)
    outbox.kick(force: true)
    try await Task.sleep(for: .milliseconds(4500))
    // Past the old window, still online and still going: "Sending…".
    #expect({ if case .sending = events.delivery.current { true } else { false } }())
    #expect(await eventually(5) { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(events.delivery.states == ["sending", "sent"])
    #expect(events.delivery.current == .sent(.delivered(sessionTitle: "Problems · Ledger")))
    #expect(events.all.first == .started(id: entry.id))
  }

  @Test func anUnsatisfiedPathIsQueuedThenSendingWhenItsBack() async throws {
    // A background session waits for the network instead of failing: the reply is late.
    StubServer.reset([.init(delay: 1.5)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    events.delivery.setPath(satisfied: false)
    events.delivery.track(entry.id)
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.started(id: entry.id)) })
    #expect(events.delivery.current == .queued)
    events.delivery.setPath(satisfied: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(events.delivery.states == ["queued", "sending", "sent"])
  }

  @Test func aFailedAttemptIsQueuedThenSendingOnRetryThenSent() async throws {
    StubServer.reset([.init(status: 503), .init(delay: 0.5)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    events.delivery.track(entry.id)
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains { if case .retrying = $0 { true } else { false } } })
    #expect(events.delivery.current == .queued)
    // The network came back (or the app was foregrounded): the retry starts now.
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.sent(id: entry.id, reply: .delivered(sessionTitle: "Problems · Ledger"))) })
    #expect(events.delivery.states == ["sending", "queued", "sending", "sent"])
  }

  @Test func otherClientErrorsFailWithoutQueueing() async throws {
    StubServer.reset([.init(status: 422)])
    let outbox = makeOutbox()
    let entry = try await outbox.enqueue(timeline: timeline, movie: movie, target: target, recordedAt: "2026-10-14T18:00:00Z")
    events.delivery.track(entry.id)
    outbox.kick(force: true)
    #expect(await eventually { events.all.contains(.failed(id: entry.id, .rejected)) })
    #expect(events.delivery.states == ["sending", "failed"])
    #expect(events.delivery.current == .failed(.rejected))
  }

  @Test func classification() {
    #expect(Outbox.classify(status: 200) == .success)
    #expect(Outbox.classify(status: 201) == .success)
    #expect(Outbox.classify(status: 401) == .permanent(.unauthorized))
    #expect(Outbox.classify(status: 403) == .permanent(.unauthorized))
    #expect(Outbox.classify(status: 413) == .permanent(.tooLarge))
    #expect(Outbox.classify(status: 500) == .retry)
    #expect(Outbox.classify(status: 429) == .retry)
    #expect(Outbox.classify(status: 408) == .retry)
    #expect(Outbox.classify(status: 400) == .permanent(.rejected))
    #expect(Outbox.classify(status: 404) == .permanent(.rejected))
    #expect(Outbox.classify(status: 411) == .permanent(.rejected))
    #expect(Outbox.classify(status: nil) == .retry)
    #expect(Outbox.reportsProgress(percent: 1, after: nil, now: Date()))
    let last = (percent: 41, at: Date(timeIntervalSinceReferenceDate: 0))
    #expect(Outbox.reportsProgress(percent: 42, after: last, now: last.at.addingTimeInterval(0.1)))
    #expect(!Outbox.reportsProgress(percent: 41, after: last, now: last.at.addingTimeInterval(4.9)))
    // Slow but moving: a heartbeat, so it doesn't look stalled.
    #expect(Outbox.reportsProgress(percent: 41, after: last, now: last.at.addingTimeInterval(5)))
    #expect(Outbox.backoff(afterAttempts: 1) == 15)
    #expect(Outbox.backoff(afterAttempts: 2) == 30)
    #expect(Outbox.backoff(afterAttempts: 20) == 1800)
  }
}

import Foundation

/// Captures in the shared container, a folder each:
/// `Captures/<id>/{capture.json, <id>.mp4, <id>.json, payload, thumbnail.jpg}`. The movie and
/// timeline share a name, as ScrawlKit's do, so `scrawl import` pairs them after an AirDrop.
/// The app and both extensions read and write them, so every change is made under a file lock
/// they all take (`.lock`), and pings the others.
final class CaptureStore: @unchecked Sendable {
  static let shared = CaptureStore(root: AppGroup.container.appending(path: "Captures", directoryHint: .isDirectory))

  let root: URL
  /// Whether changes ping the other processes (off in tests).
  let pings: Bool

  init(root: URL, pings: Bool = true) {
    self.root = root
    self.pings = pings
  }

  func folder(_ id: String) -> URL { root.appending(path: id, directoryHint: .isDirectory) }
  func movie(_ id: String) -> URL { folder(id).appending(path: "\(id).mp4") }
  func timeline(_ id: String) -> URL { folder(id).appending(path: "\(id).json") }
  /// The framed upload (4-byte big-endian timeline length, timeline JSON, movie).
  func payload(_ id: String) -> URL { folder(id).appending(path: "payload") }
  func thumbnail(_ id: String) -> URL { folder(id).appending(path: "thumbnail.jpg") }
  private func record(_ id: String) -> URL { folder(id).appending(path: "capture.json") }

  /// A new capture's folder and record. IDs sort by time, so the folder names do too.
  func create(kind: Capture.Kind, destination: Destination?, note: String? = nil, state: Capture.State, at date: Date = Date()) throws -> Capture {
    let stamp = date.formatted(.iso8601.year().month().day().dateSeparator(.omitted).time(includingFractionalSeconds: false).timeSeparator(.omitted))
    let id = "\(stamp)-\(UUID().uuidString.prefix(6).lowercased())"
    try prepareRoot()
    try FileManager.default.createDirectory(at: folder(id), withIntermediateDirectories: true)
    let capture = Capture(id: id, kind: kind, createdAt: date, destination: destination, note: note, state: state)
    try locked { try write(capture) }
    return capture
  }

  func load(_ id: String) -> Capture? {
    guard let data = try? Data(contentsOf: record(id)) else { return nil }
    return try? decoder.decode(Capture.self, from: data)
  }

  /// Every capture, newest first.
  func all() -> [Capture] {
    let names = (try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []
    return names.filter { !$0.hasPrefix(".") }.compactMap(load).sorted { $0.createdAt > $1.createdAt }
  }

  /// Changes a capture's record in place and returns it (`nil` when it's gone).
  @discardableResult
  func update(_ id: String, _ change: (inout Capture) -> Void) -> Capture? {
    transact(id) { capture in
      change(&capture)
      return true
    }
  }

  /// Reads a capture, lets `body` decide on it, and saves what `body` changed if it returns
  /// `true`, all under the lock: no other process or thread changes it in between. Returns the
  /// capture as it now is (`nil` when it's gone).
  @discardableResult
  func transact(_ id: String, _ body: (inout Capture) -> Bool) -> Capture? {
    (try? locked { () -> Capture? in
      guard let saved = load(id) else { return nil }
      var capture = saved
      guard body(&capture) else { return saved }
      capture.updatedAt = Date()
      try write(capture)
      return capture
    }) ?? nil
  }

  func delete(_ id: String) {
    try? locked { try? FileManager.default.removeItem(at: folder(id)) }
    ping()
  }

  /// Once the Mac has a capture, its movie and upload go after `mediaDays`, and every capture
  /// after `captureDays` (the Mac keeps its own copy, with the same defaults).
  func prune(now: Date = Date(), mediaDays: Double = 7, captureDays: Double = 30) {
    for capture in all() {
      let age = now.timeIntervalSince(capture.createdAt) / 86_400
      if age > captureDays, capture.state != .recording {
        try? FileManager.default.removeItem(at: folder(capture.id))
      } else if age > mediaDays, capture.isDone {
        try? FileManager.default.removeItem(at: movie(capture.id))
        try? FileManager.default.removeItem(at: payload(capture.id))
      }
    }
  }

  /// Tells the app and extensions that captures changed (they reload from disk).
  func ping() {
    if pings { Ping.capturesChanged.post() }
  }

  private func write(_ capture: Capture) throws {
    let data = try encoder.encode(capture)
    try data.write(to: record(capture.id), options: .atomic)
    ping()
  }

  /// Runs `body` holding the store's lock, an exclusive `flock` on `.lock` that the app and the
  /// extensions share (and that threads of one process contend for too: each call opens its own
  /// descriptor).
  private func locked<T>(_ body: () throws -> T) throws -> T {
    try prepareRoot()
    let descriptor = open(root.appending(path: ".lock").path, O_RDWR | O_CREAT, 0o600)
    guard descriptor >= 0 else { throw CocoaError(.fileWriteUnknown) }
    defer { close(descriptor) }
    while flock(descriptor, LOCK_EX) != 0 {
      guard errno == EINTR else { throw CocoaError(.fileLocking) }
    }
    defer { flock(descriptor, LOCK_UN) }
    return try body()
  }

  private func prepareRoot() throws {
    guard !FileManager.default.fileExists(atPath: root.path) else { return }
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    var root = root
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try? root.setResourceValues(values)
  }

  private let encoder: JSONEncoder = {
    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    return encoder
  }()

  private let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    return decoder
  }()
}

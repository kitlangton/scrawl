import Foundation
@testable import Scrawl
import Testing

struct DeliveryTests {
  let body = Data(#"{"ok":true,"delivered":true,"state":"sent","sessionTitle":"SCRAWL"}"#.utf8)

  @Test func theMacsReplySaysWhatBecameOfIt() {
    func reply(_ json: String) -> IngestReply { IngestReply(body: Data(json.utf8)) }
    #expect(reply(#"{"ok":true,"delivered":true,"state":"sent","sessionTitle":"UP DOWN"}"#) == .delivered(sessionTitle: "UP DOWN"))
    #expect(reply(#"{"ok":true,"delivered":false,"state":"queued","sessionTitle":"UP DOWN"}"#) == .queued(sessionTitle: "UP DOWN"))
    #expect(reply(#"{"ok":true,"delivered":false,"state":"processing","sessionTitle":"UP DOWN"}"#) == .processing(sessionTitle: "UP DOWN"))
    #expect(reply(#"{"ok":true,"delivered":false,"state":"saved","sessionTitle":"Mac"}"#) == .saved)
    // Macs from before `delivered` answered once the files were in.
    #expect(reply(#"{"ok":true}"#) == .delivered(sessionTitle: "OpenCode"))
  }

  @Test func outcomesAreRetriedOnlyWhenSendingAgainCanHelp() {
    func outcome(_ status: Int?, _ body: String = "{}", attempts: Int = 1) -> Uploader.Outcome {
      Uploader.outcome(status: status, body: Data(body.utf8), error: nil, attempts: attempts)
    }
    #expect(outcome(200, #"{"ok":true,"delivered":true,"state":"sent","sessionTitle":"SCRAWL"}"#) == .accepted(.delivered(sessionTitle: "SCRAWL")))
    #expect(outcome(401) == .failed(.unauthorized, nil))
    #expect(outcome(403) == .failed(.unauthorized, nil))
    #expect(outcome(413) == .failed(.tooLarge, nil))
    #expect(outcome(400, #"{"error":"Invalid framed payload"}"#) == .failed(.rejected, "Invalid framed payload"))
    #expect(outcome(429) == .retry("Your Mac was busy (429)."))
    #expect(outcome(503) == .retry("Your Mac answered 503."))
    #expect(outcome(nil) == .retry("Your Mac didn't answer."))
    // The Mac forgets a recording it couldn't build: worth a few more tries, then not.
    #expect(outcome(500, #"{"ok":false,"error":"no video track"}"#, attempts: 1) == .retry("no video track"))
    #expect(outcome(500, #"{"ok":false,"error":"no video track"}"#, attempts: 3) == .failed(.couldNotBuild, "no video track"))
    let offline = Uploader.outcome(status: nil, body: Data(), error: URLError(.notConnectedToInternet), attempts: 1)
    guard case .retry = offline else {
      Issue.record("offline should be retried, got \(offline)")
      return
    }
  }

  @Test func backoffDoublesFromFifteenSecondsToHalfAnHour() {
    #expect(Capture.backoff(afterAttempts: 0) == 15)
    #expect(Capture.backoff(afterAttempts: 1) == 15)
    #expect(Capture.backoff(afterAttempts: 2) == 30)
    #expect(Capture.backoff(afterAttempts: 4) == 120)
    #expect(Capture.backoff(afterAttempts: 20) == 1800)
  }

  @Test func thePayloadIsTheTimelinesLengthTheTimelineThenTheMovie() throws {
    let folder = FileManager.default.temporaryDirectory.appending(path: "frame-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: folder) }
    let movie = folder.appending(path: "movie.mp4")
    let movieBytes = Data((0 ..< 3_000_000).map { UInt8($0 % 251) })
    try movieBytes.write(to: movie)
    let timeline = Data(#"{"recordedAt":"2026-10-23T16:05:12.345Z"}"#.utf8)
    let payload = folder.appending(path: "payload")
    try Uploader.frame(timeline: timeline, movie: movie, into: payload)
    let framed = try Data(contentsOf: payload)
    #expect(Array(framed.prefix(4)) == [0, 0, 0, UInt8(timeline.count)])
    #expect(framed.subdata(in: 4 ..< 4 + timeline.count) == timeline)
    #expect(framed.suffix(from: 4 + timeline.count) == movieBytes)
    #expect(!FileManager.default.fileExists(atPath: payload.appendingPathExtension("partial").path))
  }

  @Test func whatIsDueToSendAgain() {
    let now = Date()
    func capture(_ state: Capture.State, next: Date? = nil, started: Date? = nil) -> Capture {
      var capture = Capture(id: "c", kind: .broadcast, createdAt: now.addingTimeInterval(-3600), destination: nil, state: state)
      capture.nextAttemptAt = next
      capture.uploadStartedAt = started
      return capture
    }
    #expect(Sender.isDue(capture(.waiting, next: now.addingTimeInterval(-1)), force: false, now: now))
    #expect(!Sender.isDue(capture(.waiting, next: now.addingTimeInterval(60)), force: false, now: now))
    #expect(Sender.isDue(capture(.waiting, next: now.addingTimeInterval(60)), force: true, now: now))
    #expect(Sender.isDue(capture(.notPaired), force: false, now: now))
    // An upload belongs to its attempt however long it takes; reconciling gives back lost ones.
    #expect(!Sender.isDue(capture(.sending, started: now.addingTimeInterval(-60)), force: true, now: now))
    #expect(!Sender.isDue(capture(.sending, started: now.addingTimeInterval(-20 * 60)), force: true, now: now))
    #expect(!Sender.isDue(capture(.accepted(.delivered(sessionTitle: "S"))), force: true, now: now))
    #expect(!Sender.isDue(capture(.failed(.unauthorized)), force: true, now: now))
  }

  @Test func oneAttemptAtATimeOwnsACaptureAndOnlyItsOutcomeCounts() throws {
    let store = CaptureStore(root: FileManager.default.temporaryDirectory.appending(path: "claims-\(UUID().uuidString)"), pings: false)
    defer { try? FileManager.default.removeItem(at: store.root) }
    let uploader = Uploader(store: store)
    let capture = try store.create(kind: .shared, destination: nil, state: .waiting)

    let first = try #require(uploader.claim(capture.id))
    #expect(first.hasSuffix(".\(capture.id).1"))
    // Sending it twice at once (the app's refresh and a retry, say): the second gets nothing.
    #expect(uploader.claim(capture.id) == nil)
    #expect(store.load(capture.id)?.state == .sending)

    // A stale attempt's news is ignored; the owner's counts.
    #expect(uploader.finish(capture.id, attempt: "\(Uploader.identifierPrefix)\(capture.id).0", outcome: .accepted(.saved)) == nil)
    #expect(store.load(capture.id)?.state == .sending)
    let waiting = try #require(uploader.finish(capture.id, attempt: first, outcome: .retry("Your Mac didn't answer.")))
    #expect(waiting.state == .waiting)
    #expect(waiting.nextAttemptAt != nil)

    let second = try #require(uploader.claim(capture.id))
    #expect(second.hasSuffix(".2"))
    #expect(uploader.finish(capture.id, attempt: first, outcome: .accepted(.saved)) == nil)
    let sent = try #require(uploader.finish(capture.id, attempt: second, outcome: .accepted(.delivered(sessionTitle: "SCRAWL"))))
    #expect(sent.state == .accepted(.delivered(sessionTitle: "SCRAWL")))
    // Done: nothing claims it again.
    #expect(uploader.claim(capture.id) == nil)
  }

  @Test func claimsHoldAcrossThreads() async throws {
    let store = CaptureStore(root: FileManager.default.temporaryDirectory.appending(path: "race-\(UUID().uuidString)"), pings: false)
    defer { try? FileManager.default.removeItem(at: store.root) }
    let uploader = Uploader(store: store)
    let capture = try store.create(kind: .broadcast, destination: nil, state: .waiting)
    let winners = await withTaskGroup(of: String?.self) { group in
      for _ in 0 ..< 16 {
        group.addTask { uploader.claim(capture.id) }
      }
      return await group.reduce(into: 0) { count, claimed in count += claimed == nil ? 0 : 1 }
    }
    #expect(winners == 1)
    #expect(store.load(capture.id)?.attempts == 1)
  }
}

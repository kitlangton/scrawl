import Foundation
@testable import Scrawl
import Testing

struct CaptureTests {
  func temporaryStore() -> CaptureStore {
    CaptureStore(root: FileManager.default.temporaryDirectory.appending(path: "captures-\(UUID().uuidString)"), pings: false)
  }

  @Test func capturesAreKeptAFolderEachNewestFirst() throws {
    let store = temporaryStore()
    defer { try? FileManager.default.removeItem(at: store.root) }
    let older = try store.create(kind: .broadcast, destination: nil, state: .recording, at: Date(timeIntervalSince1970: 1_792_000_000))
    let newer = try store.create(
      kind: .shared,
      destination: Destination(session: "ses_1", title: "SCRAWL", directory: "/Users/kit/code/scrawl"),
      note: "the badge",
      state: .waiting,
      at: Date(timeIntervalSince1970: 1_792_000_060)
    )
    #expect(store.all().map(\.id) == [newer.id, older.id])
    #expect(store.load(newer.id)?.destination?.project == "scrawl")
    // The movie and timeline share a name, so `scrawl import` pairs them.
    #expect(store.movie(newer.id).deletingPathExtension().lastPathComponent == store.timeline(newer.id).deletingPathExtension().lastPathComponent)

    let updated = store.update(older.id) {
      $0.state = .accepted(.queued(sessionTitle: "SCRAWL"))
      $0.attempts = 2
    }
    #expect(updated?.state == .accepted(.queued(sessionTitle: "SCRAWL")))
    #expect(store.load(older.id)?.attempts == 2)
    store.delete(older.id)
    #expect(store.all().map(\.id) == [newer.id])
    #expect(store.update(older.id) { $0.attempts = 9 } == nil)

    // A transaction that decides against changing anything saves nothing.
    let before = try #require(store.load(newer.id))
    let untouched = store.transact(newer.id) { capture in
      capture.attempts = 42
      return false
    }
    #expect(untouched?.attempts == 0)
    #expect(store.load(newer.id) == before)
  }

  @Test func recordedAtHasMillisecondsSoTwoInASecondArentCopiesOfOne() {
    let a = Capture(id: "a", kind: .shared, createdAt: Date(timeIntervalSince1970: 1_792_000_000.120), destination: nil, state: .waiting)
    let b = Capture(id: "b", kind: .shared, createdAt: Date(timeIntervalSince1970: 1_792_000_000.840), destination: nil, state: .waiting)
    #expect(a.recordedAt != b.recordedAt)
    #expect(a.recordedAt.contains("T") && a.recordedAt.contains(".") && a.recordedAt.hasSuffix("Z"))
  }

  @Test func mediaGoesAWeekAfterTheMacHasItAndEverythingAfterAMonth() throws {
    let store = temporaryStore()
    defer { try? FileManager.default.removeItem(at: store.root) }
    let now = Date()
    let sent = try store.create(kind: .broadcast, destination: nil, state: .accepted(.delivered(sessionTitle: "S")), at: now.addingTimeInterval(-8 * 86_400))
    let waiting = try store.create(kind: .broadcast, destination: nil, state: .waiting, at: now.addingTimeInterval(-8 * 86_400))
    let old = try store.create(kind: .broadcast, destination: nil, state: .waiting, at: now.addingTimeInterval(-31 * 86_400))
    for capture in [sent, waiting, old] {
      try Data("movie".utf8).write(to: store.movie(capture.id))
    }
    store.prune(now: now)
    #expect(!FileManager.default.fileExists(atPath: store.movie(sent.id).path))
    #expect(store.load(sent.id) != nil)
    #expect(FileManager.default.fileExists(atPath: store.movie(waiting.id).path))
    #expect(store.load(old.id) == nil)
  }

  @Test func statusLinesSayWhatHappensNext() {
    var capture = Capture(id: "c", kind: .broadcast, createdAt: Date(), destination: Destination(session: "ses_1", title: "SCRAWL"), state: .sending)
    capture.duration = 42.4
    #expect(capture.lengthText == "0:42")
    #expect(capture.statusTitle == "Sending to SCRAWL…")
    capture.state = .accepted(.delivered(sessionTitle: "SCRAWL"))
    #expect(capture.statusTitle == "Sent to SCRAWL")
    capture.state = .accepted(.queued(sessionTitle: "SCRAWL"))
    #expect(capture.statusDetail == "Sends to SCRAWL when OpenCode answers.")
    capture.screenshots = 3
    #expect(capture.lengthText == "3 screenshots")
  }

  @Test func aRecordingIsOrphanedWhenItsExtensionStopsBeating() {
    let now = Date()
    let broadcast = Capture(id: "gone", kind: .broadcast, createdAt: now.addingTimeInterval(-60), destination: nil, state: .recording)
    #expect(ActiveBroadcast.isOrphaned(broadcast, now: now))
    let fresh = Capture(id: "fresh", kind: .broadcast, createdAt: now.addingTimeInterval(-5), destination: nil, state: .recording)
    #expect(!ActiveBroadcast.isOrphaned(fresh, now: now))
    // Shared items take as long as their movie takes to make.
    let shared = Capture(id: "shared", kind: .shared, createdAt: now.addingTimeInterval(-60), destination: nil, state: .recording)
    #expect(!ActiveBroadcast.isOrphaned(shared, now: now))
  }

  @Test func theTimelineHasScrawlKitsKeysAndHowItWasRecorded() throws {
    var timeline = Timeline(recordedAt: "2026-10-23T16:05:12.345Z", duration: 12.3456, capture: .broadcast)
    timeline.screen = (402, 874, 3)
    timeline.markers = [3.21]
    timeline.events = [(4.5, "turned to landscape")]
    timeline.audio = true
    timeline.session = "ses_abc"
    timeline.note = "  "
    let json = try #require(try JSONSerialization.jsonObject(with: timeline.json()) as? [String: Any])
    #expect(json["recordedAt"] as? String == "2026-10-23T16:05:12.345Z")
    #expect(json["duration"] as? Double == 12.346)
    #expect(json["capture"] as? String == "broadcast")
    #expect(json["session"] as? String == "ses_abc")
    #expect(json["markers"] as? [Double] == [3.21])
    #expect(json["audio"] as? Bool == true)
    #expect((json["screen"] as? [String: Double])?["scale"] == 3)
    #expect((json["events"] as? [[String: Any]])?.first?["label"] as? String == "turned to landscape")
    #expect(json["device"] as? String == "iPhone")
    #expect((json["system"] as? String)?.hasPrefix("iOS ") == true)
    // A blank note, no app (the recording is of any app), no touches it can't see.
    #expect(json["note"] == nil && json["app"] == nil)
    #expect((json["touches"] as? [Any])?.isEmpty == true)

    var unaddressed = Timeline(recordedAt: "x", duration: 1, capture: .shared)
    unaddressed.screens = [(0, "Screenshot 1 of 2"), (2, "Screenshot 2 of 2")]
    let shared = try #require(try JSONSerialization.jsonObject(with: unaddressed.json()) as? [String: Any])
    #expect(shared["session"] == nil)
    #expect((shared["screens"] as? [[String: Any]])?.last?["name"] as? String == "Screenshot 2 of 2")
  }
}

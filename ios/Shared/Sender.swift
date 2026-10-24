import Foundation

/// Gets captures to the Mac: finds an address that answers and hands the upload to iOS, or says
/// why it can't. Extensions send once as they finish; the app retries what's waiting.
///
/// Only a capture that's waiting (or kept until pairing) is ever touched here: one being sent
/// belongs to its upload until it ends (`Uploader.reconcile` gives back the ones iOS lost).
enum Sender {
  /// Sends capture `id` now. With `evenIfUnreachable` (an extension's one chance), an upload
  /// starts even when no address answered, and the background session keeps at it.
  static func send(_ id: String, store: CaptureStore = .shared, pairing: Pairing? = PairingStore.load(), evenIfUnreachable: Bool) async {
    guard let pairing else {
      let kept = store.transact(id) { capture in
        guard capture.state == .waiting else { return false }
        capture.state = .notPaired
        return true
      }
      if let kept, kept.state == .notPaired { Notifier.post(kept) }
      return
    }
    start(id, reach: await MacClient.reach(pairing), pairing: pairing, store: store, evenIfUnreachable: evenIfUnreachable)
  }

  /// Hands capture `id` to the uploader once `reach` says where the Mac is.
  static func start(_ id: String, reach: MacClient.Reachability, pairing: Pairing, store: CaptureStore = .shared, evenIfUnreachable: Bool) {
    let base: URL
    switch reach {
    case let .reachable(url):
      base = url
    case .unauthorized:
      let failed = waiting(id, store: store) { $0.state = .failed(.unauthorized) }
      if let failed { Notifier.post(failed) }
      return
    case .unreachable where evenIfUnreachable:
      base = pairing.urls.first { $0.scheme == "https" } ?? pairing.urls[0]
    case .unreachable:
      var announce = false
      _ = waiting(id, store: store) { capture in
        announce = capture.state == .notPaired
        capture.state = .waiting
        capture.lastError = "Your Mac didn't answer."
        capture.nextAttemptAt = Date().addingTimeInterval(Capture.backoff(afterAttempts: max(capture.attempts, 1)))
      }
      if announce, let capture = store.load(id) { Notifier.post(capture) }
      Retry.schedule()
      return
    }
    do {
      try Uploader.shared.send(id, to: base, pairing: pairing)
      if let sending = store.load(id) { Notifier.post(sending) }
    } catch {
      // The uploader has put the capture where it belongs (waiting, failed, or someone else's).
      log.info("send \(id, privacy: .public): \(error.localizedDescription, privacy: .public)")
    }
  }

  /// Changes capture `id` only while it's waiting or kept until pairing; returns it if it did.
  private static func waiting(_ id: String, store: CaptureStore, _ change: (inout Capture) -> Void) -> Capture? {
    var changed = false
    let capture = store.transact(id) { capture in
      guard capture.state == .waiting || capture.state == .notPaired else { return false }
      change(&capture)
      changed = true
      return true
    }
    return changed ? capture : nil
  }

  /// Sends what's waiting and due (everything waiting when `force`), and what was kept until the
  /// phone was paired. One `/health` round for all.
  static func sendDue(force: Bool, store: CaptureStore = .shared, now: Date = Date()) async {
    sweep(store: store, now: now)
    guard let pairing = PairingStore.load() else { return }
    let due = store.all().filter { isDue($0, force: force, now: now) }
    guard !due.isEmpty else { return }
    let reach = await MacClient.reach(pairing)
    for capture in due {
      start(capture.id, reach: reach, pairing: pairing, store: store, evenIfUnreachable: false)
    }
  }

  static func isDue(_ capture: Capture, force: Bool, now: Date) -> Bool {
    switch capture.state {
    case .waiting: force || (capture.nextAttemptAt ?? .distantPast) <= now
    case .notPaired: true
    default: false
    }
  }

  /// The soonest a waiting capture is due, for the background refresh.
  static func nextDue(store: CaptureStore = .shared) -> Date? {
    store.all().compactMap { $0.state == .waiting ? ($0.nextAttemptAt ?? Date()) : nil }.min()
  }

  /// A new pairing: what the old one was turned down for gets another go.
  static func pairingChanged(store: CaptureStore = .shared) async {
    for capture in store.all() where capture.state == .failed(.unauthorized) {
      store.transact(capture.id) { capture in
        guard capture.state == .failed(.unauthorized) else { return false }
        capture.state = .waiting
        return true
      }
    }
    await sendDue(force: true, store: store)
  }

  /// Recordings whose extension was killed before it wrote the movie.
  static func sweep(store: CaptureStore = .shared, now: Date = Date()) {
    for capture in store.all() where ActiveBroadcast.isOrphaned(capture, now: now) {
      store.transact(capture.id) { capture in
        guard ActiveBroadcast.isOrphaned(capture, now: now) else { return false }
        capture.state = .failed(.interrupted)
        return true
      }
    }
  }
}

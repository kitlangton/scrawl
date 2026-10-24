import Foundation
import Observation

@MainActor
@Observable
final class AppModel {
  enum MacState: Equatable {
    case notPaired
    case checking
    case reachable(URL)
    case unreachable
    /// The Mac answered, but not to this pairing's token.
    case unauthorized
  }

  private(set) var pairing = PairingStore.load()
  private(set) var mac = MacState.checking
  private(set) var sessionList = DestinationStore.sessions
  private(set) var sessionsError: String?
  private(set) var isLoadingSessions = false
  private(set) var captures: [Capture] = []
  /// The capture the broadcast extension is recording.
  private(set) var recording: String?
  private(set) var isSendingTest = false
  /// Why the test capture couldn't be made.
  var testProblem: String?

  var destination = DestinationStore.current {
    didSet { DestinationStore.current = destination }
  }

  @ObservationIgnored private var observer: PingObserver?
  @ObservationIgnored private let store = CaptureStore.shared

  init() {
    reload()
    observer = Ping.capturesChanged.observe { [weak self] in
      Task { @MainActor in self?.reload() }
    }
    // `-scrawl-link <setup link>` pairs at launch: the UI tests, or pairing from Xcode.
    if let text = UserDefaults.standard.string(forKey: "scrawl-link"), let url = URL(string: text) {
      open(url)
    }
  }

  var host: String? { sessionList?.host }

  func reload() {
    captures = store.all()
    recording = ActiveBroadcast.current()
  }

  /// The app came to the front: catch up on what the extensions did, and send what's waiting.
  func refresh() async {
    Sender.sweep()
    Uploader.shared.reconcile()
    reload()
    await checkMac()
    await Sender.sendDue(force: true)
    store.prune()
    reload()
  }

  func checkMac() async {
    guard let pairing else {
      mac = .notPaired
      return
    }
    // A Mac that answered keeps saying so while it's asked again.
    if case .reachable = mac {} else { mac = .checking }
    switch await MacClient.reach(pairing) {
    case let .reachable(base):
      mac = .reachable(base)
      await loadSessions(from: base)
    case .unauthorized:
      mac = .unauthorized
    case .unreachable:
      mac = .unreachable
    }
  }

  func loadSessions() async {
    guard case let .reachable(base) = mac else {
      await checkMac()
      return
    }
    await loadSessions(from: base)
  }

  private func loadSessions(from base: URL) async {
    guard let pairing else { return }
    isLoadingSessions = true
    defer { isLoadingSessions = false }
    do {
      let list = try await MacClient.sessions(at: base, token: pairing.token)
      sessionList = list
      sessionsError = nil
      DestinationStore.sessions = list
      // Fill in the title of a destination the setup link named by ID.
      if let destination, let session = list.sessions.first(where: { $0.id == destination.session }),
         destination.title != session.title || destination.directory != session.directory
      {
        self.destination = Destination(session)
      }
    } catch {
      sessionsError = error.localizedDescription
    }
  }

  // MARK: - Pairing

  /// A setup link opened from the Camera, a QR code scanned here, or pasted. `false` when it
  /// isn't one.
  @discardableResult
  func open(_ url: URL) -> Bool {
    guard let link = Pairing.parse(url) else { return false }
    apply(link)
    return true
  }

  @discardableResult
  func paste(_ text: String) -> Bool {
    guard let link = Pairing.parse(text: text) else { return false }
    apply(link)
    return true
  }

  private func apply(_ link: Pairing.Link) {
    switch link {
    case let .pair(pairing):
      PairingStore.save(pairing)
      let changed = pairing != self.pairing
      self.pairing = pairing
      if let session = pairing.session, destination?.session != session {
        destination = Destination(session: session, title: session)
      }
      mac = .checking
      Task {
        await Notifier.requestAuthorization()
        await checkMac()
        if changed { await Sender.pairingChanged() }
        reload()
      }
    case .reset:
      unpair()
    }
  }

  func unpair() {
    PairingStore.clear()
    pairing = nil
    mac = .notPaired
    sessionList = nil
    sessionsError = nil
    DestinationStore.sessions = nil
  }

  // MARK: - Captures

  func sendTest() async {
    isSendingTest = true
    defer { isSendingTest = false }
    do {
      let capture = try await SharedCapture.test(destination: destination)
      await Sender.send(capture.id, evenIfUnreachable: false)
    } catch {
      testProblem = error.localizedDescription
    }
    reload()
  }

  func retry(_ capture: Capture) async {
    store.transact(capture.id) { capture in
      guard capture.canRetry else { return false }
      // Another round of build attempts for one the Mac couldn't build.
      if case .failed = capture.state { capture.attempts = 0 }
      capture.state = .waiting
      capture.nextAttemptAt = nil
      return true
    }
    await Sender.sendDue(force: true)
    reload()
  }

  func delete(_ capture: Capture) {
    store.delete(capture.id)
    reload()
  }
}

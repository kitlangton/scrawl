import CoreMedia
import Foundation
import ImageIO

/// One screen recording, from the broadcast starting to its upload being handed to iOS. The
/// broadcast extension feeds it ReplayKit's buffers; the capture, movie and timeline go in the
/// shared container, so the app lists it and finishes sending it if the extension can't.
final class BroadcastSession: @unchecked Sendable {
  let capture: Capture
  let store: CaptureStore
  let pairing: Pairing?
  /// The longest recording the Mac (or the HTTPS endpoint) takes at this bitrate.
  let limit: TimeInterval
  /// Called once when the recording reaches `limit`; the extension ends the broadcast.
  var onLimit: (@Sendable () -> Void)?

  private let recorder: BroadcastRecorder
  private let lock = NSLock()
  private var flags: [Double] = []
  private var events: [(t: Double, label: String)] = []
  private var ended = false
  private var limitReached = false
  private var flagObserver: PingObserver?
  private var heartbeat: DispatchSourceTimer?
  private var reachability: Task<MacClient.Reachability, Never>?

  /// 4 minutes at ~0.76 MB a second is ~180 MB of the Mac's 250; 3 minutes at half that fits an
  /// HTTPS endpoint's 95.
  static func limit(hosted: Bool) -> TimeInterval { hosted ? 180 : 240 }

  init(
    store: CaptureStore = .shared,
    pairing: Pairing? = PairingStore.load(),
    destination: Destination? = DestinationStore.current,
    now: Date = Date()
  ) throws {
    self.store = store
    self.pairing = pairing
    let hosted = pairing?.isHosted ?? false
    limit = Self.limit(hosted: hosted)
    let destination = destination ?? pairing?.session.map { Destination(session: $0, title: $0) }
    capture = try store.create(kind: .broadcast, destination: destination, state: .recording, at: now)
    recorder = BroadcastRecorder(url: store.movie(capture.id), settings: .forPairing(hosted: hosted))
    ActiveBroadcast.begin(capture.id)
    // Find the Mac while recording, so the upload can leave the moment it stops.
    if let pairing {
      reachability = Task { await MacClient.reach(pairing) }
    }
    flagObserver = Ping.flag.observe { [weak self] in self?.flag() }
    startHeartbeat()
    log.info("broadcast \(self.capture.id, privacy: .public) started")
  }

  func video(_ sample: CMSampleBuffer) {
    recorder.appendVideo(sample)
    guard let elapsed = recorder.elapsed(), elapsed >= limit else { return }
    let first = lock.withLock { () -> Bool in
      defer { limitReached = true }
      return !limitReached
    }
    if first { onLimit?() }
  }

  func microphone(_ sample: CMSampleBuffer) {
    recorder.appendAudio(sample)
  }

  /// Flags this moment ("look here"): it gets a keyframe and a `⚑`.
  func flag() {
    guard let t = recorder.elapsed() else { return }
    lock.withLock { flags.append(t) }
    log.info("flag at \(t)")
  }

  func paused() {
    guard let t = recorder.elapsed() else { return }
    lock.withLock { events.append((t, "recording paused")) }
  }

  func resumed() {
    guard let t = recorder.elapsed() else { return }
    lock.withLock { events.append((t, "recording resumed")) }
  }

  /// Ends the recording: writes the movie and its timeline and starts the upload. Blocks until
  /// they're saved, as ReplayKit's `broadcastFinished` must.
  func finish() {
    let first = lock.withLock { () -> Bool in
      defer { ended = true }
      return !ended
    }
    guard first else { return }
    heartbeat?.cancel()
    flagObserver = nil
    defer { ActiveBroadcast.end() }

    let movie: BroadcastRecorder.Movie
    do {
      movie = try recorder.finish()
    } catch BroadcastRecorder.RecorderError.noFrames {
      // Stopped before the first frame: nothing to keep.
      store.delete(capture.id)
      return
    } catch {
      log.error("broadcast \(self.capture.id, privacy: .public) couldn't be written: \(error.localizedDescription, privacy: .public)")
      if let failed = store.update(capture.id, {
        $0.state = .failed(.interrupted)
        $0.lastError = error.localizedDescription
      }) { Notifier.post(failed) }
      return
    }

    var timeline = Timeline(recordedAt: capture.recordedAt, duration: movie.duration, capture: .broadcast)
    timeline.screen = Self.screen(pixels: recorder.sourceSize)
    timeline.audio = movie.audio
    timeline.session = capture.destination?.session
    lock.withLock {
      timeline.markers = flags
      timeline.events = events
    }
    for turn in recorder.rotations {
      timeline.events.append((turn.t, BroadcastRecorder.isLandscape(turn.orientation) ? "turned to landscape" : "turned to portrait"))
    }
    timeline.events.sort { $0.t < $1.t }
    do {
      try timeline.json().write(to: store.timeline(capture.id), options: .atomic)
    } catch {
      store.update(capture.id) {
        $0.state = .failed(.interrupted)
        $0.lastError = error.localizedDescription
      }
      return
    }
    // Saved: from here on it's a capture waiting to be sent, which the app can send if this
    // extension is stopped before it hands the upload over.
    let saved = store.update(capture.id) {
      $0.duration = movie.duration
      $0.state = pairing == nil ? .notPaired : .waiting
    }
    log.info("broadcast \(self.capture.id, privacy: .public) recorded \(movie.duration)s, \(movie.frames) frames, audio \(movie.audio)")
    guard let pairing else {
      if let saved { Notifier.post(saved) }
      return
    }
    // To the address that answered while recording, else the first one: the background session
    // keeps at it, and the app tries again later if it fails.
    let reach = reachability.flatMap { Self.wait(for: $0, timeout: 2) } ?? .unreachable
    Sender.start(capture.id, reach: reach, pairing: pairing, store: store, evenIfUnreachable: true)
  }

  private func startHeartbeat() {
    let timer = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
    timer.schedule(deadline: .now() + 5, repeating: 5)
    timer.setEventHandler { ActiveBroadcast.beat() }
    timer.resume()
    heartbeat = timer
  }

  /// The screen in points, from the frames' native pixels: phones are @3x unless that would make
  /// them narrower than any iPhone (then @2x).
  static func screen(pixels: CGSize?) -> (width: Double, height: Double, scale: Double)? {
    guard let pixels, pixels.width > 0, pixels.height > 0 else { return nil }
    let short = min(pixels.width, pixels.height)
    let scale: Double = (320 ... 440).contains(short / 3) ? 3 : 2
    return (pixels.width / scale, pixels.height / scale, scale)
  }

  /// Waits (blocking) up to `timeout` for `task`.
  private static func wait<T: Sendable>(for task: Task<T, Never>, timeout: TimeInterval) -> T? {
    let box = Box<T>()
    let done = DispatchSemaphore(value: 0)
    Task {
      box.value = await task.value
      done.signal()
    }
    return done.wait(timeout: .now() + timeout) == .success ? box.value : nil
  }

  static func limitError(_ limit: TimeInterval) -> NSError {
    let minutes = Int(limit / 60)
    return NSError(domain: "dev.kitlangton.scrawl", code: 1, userInfo: [
      NSLocalizedDescriptionKey: "Scrawl records \(minutes) minutes at most. What you recorded is on its way to your Mac.",
    ])
  }
}

private final class Box<T>: @unchecked Sendable {
  var value: T?
}

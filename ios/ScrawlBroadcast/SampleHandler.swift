import ReplayKit

/// Records the whole screen (whatever app is in front) with the microphone, from Control
/// Center's Screen Recording or the app's Record button, and sends it to the Mac when it stops.
final class SampleHandler: RPBroadcastSampleHandler, @unchecked Sendable {
  private let lock = NSLock()
  private var session: BroadcastSession?

  override func broadcastStarted(withSetupInfo _: [String: NSObject]?) {
    do {
      let session = try BroadcastSession()
      session.onLimit = { [weak self, weak session] in
        guard let session else { return }
        session.finish()
        self?.finishBroadcastWithError(BroadcastSession.limitError(session.limit))
      }
      lock.withLock { self.session = session }
    } catch {
      finishBroadcastWithError(error)
    }
  }

  override func processSampleBuffer(_ sampleBuffer: CMSampleBuffer, with type: RPSampleBufferType) {
    guard let session = lock.withLock({ self.session }) else { return }
    switch type {
    case .video: session.video(sampleBuffer)
    case .audioMic: session.microphone(sampleBuffer)
    // Other apps' sound would drown out the narration the Mac transcribes.
    case .audioApp: break
    @unknown default: break
    }
  }

  override func broadcastPaused() {
    lock.withLock { session }?.paused()
  }

  override func broadcastResumed() {
    lock.withLock { session }?.resumed()
  }

  override func broadcastFinished() {
    lock.withLock { session }?.finish()
  }
}

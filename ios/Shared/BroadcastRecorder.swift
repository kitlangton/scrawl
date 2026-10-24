import AVFoundation
import CoreMedia
import ImageIO
import ReplayKit

/// Writes a ReplayKit broadcast's screen and microphone into an H.264/AAC `.mp4`, as ScrawlKit
/// records its movies (so the Mac reads it the same way). t=0 is the first frame.
///
/// ReplayKit calls in on its own threads; every writer call is serialized on one queue. Nothing
/// holds a sample buffer past its append, which keeps the extension well under its memory limit.
final class BroadcastRecorder: @unchecked Sendable {
  struct Settings: Sendable {
    /// Frames are scaled to fit this long edge (the Mac's keyframes are 1600 px).
    var maxLongEdge = 1920
    var videoBitRate = 6_000_000
    var framesPerSecond = 30
    var audioBitRate = 96_000

    /// ~0.76 MB a second for a Mac; half that for an HTTPS endpoint's 95 MB.
    static func forPairing(hosted: Bool) -> Settings {
      var settings = Settings()
      if hosted { settings.videoBitRate = 3_000_000 }
      return settings
    }
  }

  struct Movie: Sendable {
    var duration: TimeInterval
    /// The movie's frame size in pixels, as recorded (before the orientation transform).
    var size: CGSize
    var audio: Bool
    var frames: Int
  }

  enum RecorderError: Error, LocalizedError {
    case noFrames
    case writer(String)

    var errorDescription: String? {
      switch self {
      case .noFrames: "No frames were recorded."
      case let .writer(message): message
      }
    }
  }

  let url: URL
  let settings: Settings
  private let queue = DispatchQueue(label: "dev.kitlangton.scrawl.broadcast.writer", qos: .userInitiated)
  private var writer: AVAssetWriter?
  private var video: AVAssetWriterInput?
  private var audio: AVAssetWriterInput?
  private var start = CMTime.invalid
  private var lastVideo = CMTime.invalid
  /// Where the last microphone buffer taken ends. ReplayKit sends no frames while the screen is
  /// still, so narration can run well past the last frame.
  private var audioEnd = CMTime.invalid
  private var frames = 0
  private var audioSamples = 0
  private var failure: String?
  private var finished = false
  /// When the first frame arrived, for times that don't come with a buffer (flags).
  private var firstFrameAt: Date?
  private var orientation: CGImagePropertyOrientation?
  private var turns: [(t: Double, orientation: CGImagePropertyOrientation)] = []
  private var source: CGSize?

  /// The first frame's size in pixels: the screen's native resolution.
  var sourceSize: CGSize? { queue.sync { source } }

  init(url: URL, settings: Settings = Settings()) {
    self.url = url
    self.settings = settings
  }

  /// Seconds since the first frame, now; `nil` before it.
  func elapsed(at date: Date = Date()) -> Double? {
    queue.sync { firstFrameAt.map { max(0, date.timeIntervalSince($0)) } }
  }

  /// Times (seconds from the first frame) the screen turned after the first frame, and to what.
  var rotations: [(t: Double, orientation: CGImagePropertyOrientation)] { queue.sync { turns } }

  func appendVideo(_ sample: CMSampleBuffer) {
    guard CMSampleBufferDataIsReady(sample), let pixels = CMSampleBufferGetImageBuffer(sample) else { return }
    let time = CMSampleBufferGetPresentationTimeStamp(sample)
    let frameOrientation = Self.orientation(of: sample)
    queue.sync {
      guard !finished, failure == nil else { return }
      if writer == nil {
        begin(width: CVPixelBufferGetWidth(pixels), height: CVPixelBufferGetHeight(pixels), orientation: frameOrientation, at: time)
      }
      guard let video, start.isValid, time >= start else { return }
      if lastVideo.isValid {
        // At most `framesPerSecond`: ReplayKit can deliver 60 or more.
        let gap = CMTimeGetSeconds(CMTimeSubtract(time, lastVideo))
        guard gap >= 1 / Double(settings.framesPerSecond) - 0.002 else { return }
      }
      if let frameOrientation, frameOrientation != orientation {
        orientation = frameOrientation
        turns.append((CMTimeGetSeconds(CMTimeSubtract(time, start)), frameOrientation))
      }
      guard video.isReadyForMoreMediaData else { return }
      if video.append(sample) {
        lastVideo = time
        frames += 1
      } else if let error = writer?.error {
        failure = error.localizedDescription
      }
    }
  }

  func appendAudio(_ sample: CMSampleBuffer) {
    guard CMSampleBufferDataIsReady(sample) else { return }
    let time = CMSampleBufferGetPresentationTimeStamp(sample)
    let end = CMTimeAdd(time, Self.duration(of: sample))
    queue.sync {
      guard !finished, failure == nil, let audio, start.isValid, time >= start, audio.isReadyForMoreMediaData else { return }
      if audio.append(sample) {
        audioSamples += 1
        if !audioEnd.isValid || end > audioEnd { audioEnd = end }
      }
    }
  }

  /// An audio buffer's length: its own duration, else its sample count at its rate.
  static func duration(of sample: CMSampleBuffer) -> CMTime {
    let duration = CMSampleBufferGetDuration(sample)
    if duration.isValid, duration > .zero { return duration }
    guard let format = CMSampleBufferGetFormatDescription(sample),
          let rate = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee.mSampleRate, rate > 0
    else { return .zero }
    return CMTime(value: CMTimeValue(CMSampleBufferGetNumSamples(sample)), timescale: CMTimeScale(rate))
  }

  /// Finishes the movie (waiting up to `timeout`) and says what it holds.
  func finish(timeout: TimeInterval = 15) throws -> Movie {
    let (writer, lastFrame, audioEnd, frames) = queue.sync { () -> (AVAssetWriter?, CMTime, CMTime, Int) in
      finished = true
      if self.frames > 0 {
        video?.markAsFinished()
        audio?.markAsFinished()
      }
      return (self.writer, lastVideo, self.audioEnd, self.frames)
    }
    guard let writer, frames > 0, lastFrame.isValid else {
      writer?.cancelWriting()
      try? FileManager.default.removeItem(at: url)
      throw RecorderError.noFrames
    }
    // The last frame holds a moment (the movie shouldn't end the instant it appeared), and on
    // through whatever was said after it: the screen just didn't change.
    var end = CMTimeAdd(lastFrame, CMTime(value: 1, timescale: CMTimeScale(settings.framesPerSecond)))
    if audioEnd.isValid, audioEnd > end { end = audioEnd }
    writer.endSession(atSourceTime: end)
    let done = DispatchSemaphore(value: 0)
    writer.finishWriting { done.signal() }
    guard done.wait(timeout: .now() + timeout) == .success else {
      throw RecorderError.writer("The movie took too long to write.")
    }
    guard writer.status == .completed else {
      throw RecorderError.writer(writer.error?.localizedDescription ?? failure ?? "The movie couldn't be written.")
    }
    return queue.sync {
      Movie(
        duration: CMTimeGetSeconds(CMTimeSubtract(end, start)),
        size: CGSize(width: video?.outputSettings?[AVVideoWidthKey] as? Int ?? 0, height: video?.outputSettings?[AVVideoHeightKey] as? Int ?? 0),
        audio: audioSamples > 0,
        frames: frames
      )
    }
  }

  /// Throws the movie away.
  func cancel() {
    queue.sync {
      finished = true
      writer?.cancelWriting()
    }
    try? FileManager.default.removeItem(at: url)
  }

  // MARK: - Setup

  /// Sets the writer up for frames of `width`×`height`, which the first frame settles. Audio
  /// gets an input from the start: if the microphone is off, it gets no samples and the movie
  /// has no audio track.
  private func begin(width: Int, height: Int, orientation: CGImagePropertyOrientation?, at time: CMTime) {
    do {
      try? FileManager.default.removeItem(at: url)
      let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
      writer.shouldOptimizeForNetworkUse = false
      let size = Self.outputSize(width: width, height: height, maxLongEdge: settings.maxLongEdge)
      let video = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: Int(size.width),
        AVVideoHeightKey: Int(size.height),
        AVVideoScalingModeKey: AVVideoScalingModeResizeAspect,
        AVVideoColorPropertiesKey: [
          AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
          AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
          AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
        ],
        AVVideoCompressionPropertiesKey: [
          AVVideoAverageBitRateKey: settings.videoBitRate,
          AVVideoExpectedSourceFrameRateKey: settings.framesPerSecond,
          AVVideoMaxKeyFrameIntervalKey: settings.framesPerSecond,
          AVVideoMaxKeyFrameIntervalDurationKey: 1.0,
          AVVideoAllowFrameReorderingKey: false,
          AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
        ],
      ])
      video.expectsMediaDataInRealTime = true
      video.transform = Self.transform(for: orientation)
      guard writer.canAdd(video) else { throw RecorderError.writer("The video input was turned down.") }
      writer.add(video)
      let audio = AVAssetWriterInput(mediaType: .audio, outputSettings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC,
        AVNumberOfChannelsKey: 1,
        AVSampleRateKey: 48_000,
        AVEncoderBitRateKey: settings.audioBitRate,
      ])
      audio.expectsMediaDataInRealTime = true
      if writer.canAdd(audio) {
        writer.add(audio)
        self.audio = audio
      }
      guard writer.startWriting() else {
        throw RecorderError.writer(writer.error?.localizedDescription ?? "The movie couldn't be started.")
      }
      writer.startSession(atSourceTime: time)
      self.writer = writer
      self.video = video
      start = time
      firstFrameAt = Date()
      source = CGSize(width: width, height: height)
      self.orientation = orientation
    } catch {
      failure = error.localizedDescription
    }
  }

  /// Fits `width`×`height` into `maxLongEdge`, in even numbers (H.264 wants them).
  static func outputSize(width: Int, height: Int, maxLongEdge: Int) -> CGSize {
    let long = max(width, height)
    let scale = long > maxLongEdge ? Double(maxLongEdge) / Double(long) : 1
    let even = { (value: Double) in max(2, Int((value / 2).rounded()) * 2) }
    return CGSize(width: even(Double(width) * scale), height: even(Double(height) * scale))
  }

  /// ReplayKit's frames stay in the screen's native (portrait) layout and say how they're turned.
  static func orientation(of sample: CMSampleBuffer) -> CGImagePropertyOrientation? {
    guard let number = CMGetAttachment(sample, key: RPVideoSampleOrientationKey as CFString, attachmentModeOut: nil) as? NSNumber else {
      return nil
    }
    return CGImagePropertyOrientation(rawValue: number.uint32Value)
  }

  /// The track transform that shows frames turned `orientation` upright (EXIF semantics: `.right`
  /// is turned a quarter counterclockwise, so it plays a quarter clockwise).
  static func transform(for orientation: CGImagePropertyOrientation?) -> CGAffineTransform {
    switch orientation {
    case .right: CGAffineTransform(rotationAngle: .pi / 2)
    case .left: CGAffineTransform(rotationAngle: -.pi / 2)
    case .down: CGAffineTransform(rotationAngle: .pi)
    default: .identity
    }
  }

  static func isLandscape(_ orientation: CGImagePropertyOrientation) -> Bool {
    [.left, .right, .leftMirrored, .rightMirrored].contains(orientation)
  }
}

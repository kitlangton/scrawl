@preconcurrency import AVFoundation
import CoreVideo
import Foundation
import UniformTypeIdentifiers

/// Explicit encoder color metadata. No display or source color space is inferred here.
struct VideoColorProperties {
  let primaries: String
  let transferFunction: String
  let ycbcrMatrix: String

  static let rec709 = VideoColorProperties(
    primaries: AVVideoColorPrimaries_ITU_R_709_2,
    transferFunction: AVVideoTransferFunction_ITU_R_709_2,
    ycbcrMatrix: AVVideoYCbCrMatrix_ITU_R_709_2
  )
}

/// Optional pixel-buffer pool requirements. Appended buffers must still match the configured
/// dimensions and format; this setting never converts or relabels their contents.
struct VideoPixelBufferPool {
  let pixelFormat: OSType
  let usesIOSurface: Bool
}

/// Encoder choices shared by the legacy file exporter and the byte-sink writer.
struct VideoEncodingConfiguration {
  let codec: AVVideoCodecType
  let width: Int
  let height: Int
  let averageBitRate: Int
  let maxKeyFrameIntervalDuration: Int
  let expectedSourceFrameRate: Int?
  let allowsFrameReordering: Bool
  let expectsMediaDataInRealTime: Bool
  let profileLevel: String?
  let colorProperties: VideoColorProperties?
  let pixelBufferPool: VideoPixelBufferPool?

  func makeInput() -> (AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor) {
    var compression: [String: Any] = [
      AVVideoAverageBitRateKey: averageBitRate,
      AVVideoMaxKeyFrameIntervalDurationKey: maxKeyFrameIntervalDuration,
      AVVideoAllowFrameReorderingKey: allowsFrameReordering,
    ]
    if let expectedSourceFrameRate {
      compression[AVVideoExpectedSourceFrameRateKey] = expectedSourceFrameRate
    }
    if let profileLevel { compression[AVVideoProfileLevelKey] = profileLevel }

    var settings: [String: Any] = [
      AVVideoCodecKey: codec,
      AVVideoWidthKey: width,
      AVVideoHeightKey: height,
      AVVideoCompressionPropertiesKey: compression,
    ]
    if let colorProperties {
      settings[AVVideoColorPropertiesKey] = [
        AVVideoColorPrimariesKey: colorProperties.primaries,
        AVVideoTransferFunctionKey: colorProperties.transferFunction,
        AVVideoYCbCrMatrixKey: colorProperties.ycbcrMatrix,
      ]
    }

    let input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
    input.expectsMediaDataInRealTime = expectsMediaDataInRealTime
    let poolAttributes: [String: Any]? = pixelBufferPool.map { pool in
      var attributes: [String: Any] = [
        kCVPixelBufferPixelFormatTypeKey as String: pool.pixelFormat,
        kCVPixelBufferWidthKey as String: width,
        kCVPixelBufferHeightKey as String: height,
      ]
      if pool.usesIOSurface {
        attributes[kCVPixelBufferIOSurfacePropertiesKey as String] = [String: Any]()
      }
      return attributes
    }
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(
      assetWriterInput: input,
      sourcePixelBufferAttributes: poolAttributes
    )
    return (input, adaptor)
  }
}

enum SegmentedVideoWriterError: Error, Equatable {
  case invalidConfiguration
  case invalidTime
  case timeBeforeOrigin
  case invalidPixelBuffer
  case appendFailed(String)
  case byteLimitExceeded
  case segmentLimitExceeded
  case sinkFailed(String)
  case noSamples
  case alreadyFinished
  case writerFailed(String)
}

struct SegmentedVideoWriterCompletion: Equatable {
  let segments: Int
  let bytes: Int
  let appendedSamples: Int
}

private final class SegmentDataSink: NSObject, AVAssetWriterDelegate, @unchecked Sendable {
  private let lock = NSLock()
  private let byteLimit: Int
  private let segmentLimit: Int
  private let sink: (Data, AVAssetSegmentType) throws -> Void
  private var byteCount = 0
  private var segmentCount = 0
  private var failure: SegmentedVideoWriterError?

  init(
    byteLimit: Int,
    segmentLimit: Int,
    sink: @escaping (Data, AVAssetSegmentType) throws -> Void
  ) {
    self.byteLimit = byteLimit
    self.segmentLimit = segmentLimit
    self.sink = sink
  }

  func assetWriter(
    _ writer: AVAssetWriter,
    didOutputSegmentData segmentData: Data,
    segmentType: AVAssetSegmentType,
    segmentReport: AVAssetSegmentReport?
  ) {
    lock.lock()
    defer { lock.unlock() }
    guard failure == nil else { return }
    let (nextSegments, segmentsOverflow) = segmentCount.addingReportingOverflow(1)
    let (nextBytes, bytesOverflow) = byteCount.addingReportingOverflow(segmentData.count)
    guard !segmentsOverflow, nextSegments <= segmentLimit else {
      failure = .segmentLimitExceeded
      return
    }
    guard !bytesOverflow, nextBytes <= byteLimit else {
      failure = .byteLimitExceeded
      return
    }
    do {
      // Serialize sink calls in AVFoundation callback order. The sink must not call back into
      // this writer; it should synchronously write only the provided bytes to its owned sink.
      try sink(segmentData, segmentType)
      segmentCount = nextSegments
      byteCount = nextBytes
    } catch {
      failure = .sinkFailed(String(describing: error))
    }
  }

  func currentFailure() -> SegmentedVideoWriterError? {
    lock.lock()
    defer { lock.unlock() }
    return failure
  }

  func totals() -> (segments: Int, bytes: Int) {
    lock.lock()
    defer { lock.unlock() }
    return (segmentCount, byteCount)
  }
}

/// A single-use AVAssetWriter that sends fragmented-MP4 segment data to a synchronous caller
/// sink. It never resolves, creates, changes, or removes a path. Callers must serialize append
/// and finish, pass buffers with the configured dimensions/format, and retain their sink for the
/// writer's lifetime. Pixel storage/layout and color interpretation must already be valid for the
/// explicit encoding settings; this adapter performs no conversion or inferred color labeling.
/// The successful result means only AVAssetWriter completed and every emitted
/// segment reached the sink; it does not validate a conventional MP4 container or encoded origin.
final class SegmentedVideoWriter {
  private let writer: AVAssetWriter
  private let input: AVAssetWriterInput
  private let adaptor: AVAssetWriterInputPixelBufferAdaptor
  private let configuration: VideoEncodingConfiguration
  private let timeOrigin: CMTime
  private let segmentSink: SegmentDataSink
  private var finishAttempted = false
  private var appendedSamples = 0
  private var appendFailure: SegmentedVideoWriterError?

  init(
    configuration: VideoEncodingConfiguration,
    timeOrigin: CMTime,
    preferredSegmentInterval: CMTime,
    maximumBytes: Int,
    maximumSegments: Int,
    sink: @escaping (Data, AVAssetSegmentType) throws -> Void
  ) throws {
    guard configuration.width > 0, configuration.height > 0,
          configuration.averageBitRate > 0,
          configuration.maxKeyFrameIntervalDuration > 0,
          configuration.expectedSourceFrameRate.map({ $0 > 0 }) ?? true,
          maximumBytes > 0, maximumSegments > 0,
          timeOrigin.isNumeric, preferredSegmentInterval.isNumeric,
          CMTimeCompare(preferredSegmentInterval, .zero) > 0
    else { throw SegmentedVideoWriterError.invalidConfiguration }

    guard timeOrigin.timescale > 0 else { throw SegmentedVideoWriterError.invalidTime }
    guard let contentType = UTType(AVFileType.mp4.rawValue) else {
      throw SegmentedVideoWriterError.invalidConfiguration
    }
    let writer = AVAssetWriter(contentType: contentType)
    // The URL-free segmented route needs an explicit fragmented-MP4 file profile.
    writer.outputFileTypeProfile = .mpeg4AppleHLS
    let (input, adaptor) = configuration.makeInput()
    guard writer.canAdd(input) else {
      throw SegmentedVideoWriterError.writerFailed("AVAssetWriter refused video input")
    }
    let segmentSink = SegmentDataSink(
      byteLimit: maximumBytes,
      segmentLimit: maximumSegments,
      sink: sink
    )
    writer.delegate = segmentSink
    writer.preferredOutputSegmentInterval = preferredSegmentInterval
    writer.initialSegmentStartTime = .zero
    writer.add(input)
    guard writer.startWriting() else {
      throw SegmentedVideoWriterError.writerFailed(
        writer.error?.localizedDescription ?? "AVAssetWriter failed to start"
      )
    }
    writer.startSession(atSourceTime: .zero)
    self.writer = writer
    self.input = input
    self.adaptor = adaptor
    self.configuration = configuration
    self.timeOrigin = timeOrigin
    self.segmentSink = segmentSink
  }

  /// Appends the supplied pixels at their exact caller timestamp relative to `timeOrigin`.
  /// Returns false only for AVAssetWriter backpressure; it does not queue or retry the sample.
  @discardableResult
  func append(_ pixelBuffer: CVPixelBuffer, at presentationTime: CMTime) throws -> Bool {
    guard !finishAttempted, appendFailure == nil else {
      throw appendFailure ?? SegmentedVideoWriterError.alreadyFinished
    }
    if let failure = segmentSink.currentFailure() { throw failure }
    guard presentationTime.isNumeric, presentationTime.timescale > 0,
          presentationTime.epoch == timeOrigin.epoch
    else { return try failAppend(.invalidTime) }
    let relativeTime = CMTimeSubtract(presentationTime, timeOrigin)
    guard relativeTime.isNumeric, CMTimeCompare(relativeTime, .zero) >= 0 else {
      return try failAppend(.timeBeforeOrigin)
    }
    guard CVPixelBufferGetWidth(pixelBuffer) == configuration.width,
          CVPixelBufferGetHeight(pixelBuffer) == configuration.height,
          configuration.pixelBufferPool.map({
            CVPixelBufferGetPixelFormatType(pixelBuffer) == $0.pixelFormat
          }) ?? true
    else { return try failAppend(.invalidPixelBuffer) }
    guard input.isReadyForMoreMediaData else { return false }
    guard adaptor.append(pixelBuffer, withPresentationTime: relativeTime) else {
      return try failAppend(.appendFailed(writer.error?.localizedDescription ?? "pixel append failed"))
    }
    appendedSamples += 1
    return true
  }

  /// Completes once. A sink/append failure cancels this incomplete writer and returns an error;
  /// it never returns completion totals for a partial segment stream.
  func finish() async throws -> SegmentedVideoWriterCompletion {
    guard !finishAttempted else { throw SegmentedVideoWriterError.alreadyFinished }
    finishAttempted = true
    if let failure = appendFailure ?? segmentSink.currentFailure() {
      writer.cancelWriting()
      throw failure
    }
    guard appendedSamples > 0 else {
      writer.cancelWriting()
      throw SegmentedVideoWriterError.noSamples
    }
    input.markAsFinished()
    await writer.finishWriting()
    if let failure = appendFailure ?? segmentSink.currentFailure() { throw failure }
    guard writer.status == .completed else {
      throw SegmentedVideoWriterError.writerFailed(
        writer.error?.localizedDescription ?? "AVAssetWriter did not complete"
      )
    }
    let totals = segmentSink.totals()
    guard totals.segments > 0, totals.bytes > 0 else {
      throw SegmentedVideoWriterError.writerFailed("AVAssetWriter emitted no segment data")
    }
    return SegmentedVideoWriterCompletion(
      segments: totals.segments,
      bytes: totals.bytes,
      appendedSamples: appendedSamples
    )
  }

  private func failAppend(_ error: SegmentedVideoWriterError) throws -> Bool {
    appendFailure = error
    throw error
  }
}

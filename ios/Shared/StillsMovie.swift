import AVFoundation
import CoreGraphics
import CoreVideo
import ImageIO

/// Screenshots as a short movie, so the Mac builds a bundle from them as from any phone
/// recording: each holds for `hold` seconds on a canvas sized for the largest. Each is decoded
/// only while it's written, so ten screenshots fit in the share sheet's memory.
enum StillsMovie {
  static let hold: TimeInterval = 2
  /// The Mac's keyframes fit 1600 px; screenshots are drawn a little larger.
  static let maxLongEdge = 1920

  struct Movie: Sendable {
    var duration: TimeInterval
    var size: CGSize
  }

  enum StillsError: Error, LocalizedError {
    case unreadable

    var errorDescription: String? { "The screenshots couldn't be read." }
  }

  static func write(_ files: [URL], to url: URL) async throws -> Movie {
    let sources = files.compactMap { CGImageSourceCreateWithURL($0 as CFURL, nil) }
    let sizes = sources.compactMap(pixelSize)
    guard !sources.isEmpty, sizes.count == sources.count,
          let largest = sizes.max(by: { $0.width * $0.height < $1.width * $1.height })
    else { throw StillsError.unreadable }
    let size = BroadcastRecorder.outputSize(width: Int(largest.width), height: Int(largest.height), maxLongEdge: maxLongEdge)
    try? FileManager.default.removeItem(at: url)
    let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: AVVideoCodecType.h264,
      AVVideoWidthKey: Int(size.width),
      AVVideoHeightKey: Int(size.height),
      AVVideoCompressionPropertiesKey: [
        // A frame per screenshot, each a keyframe with bits to spare: text stays sharp.
        AVVideoAverageBitRateKey: 8_000_000,
        AVVideoMaxKeyFrameIntervalKey: 1,
        AVVideoAllowFrameReorderingKey: false,
        AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
      ],
    ])
    input.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
      kCVPixelBufferWidthKey as String: Int(size.width),
      kCVPixelBufferHeightKey as String: Int(size.height),
      kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any](),
    ])
    guard writer.canAdd(input) else { throw StillsError.unreadable }
    writer.add(input)
    guard writer.startWriting() else { throw writer.error ?? StillsError.unreadable }
    writer.startSession(atSourceTime: .zero)
    var done = false
    defer {
      if !done {
        writer.cancelWriting()
        try? FileManager.default.removeItem(at: url)
      }
    }

    for (index, source) in sources.enumerated() {
      // The encoder takes a frame within milliseconds; one that stopped (a full disk, a
      // failure) never gets ready again.
      let deadline = Date().addingTimeInterval(10)
      while !input.isReadyForMoreMediaData {
        guard writer.status == .writing, Date() < deadline else { throw writer.error ?? StillsError.unreadable }
        try await Task.sleep(for: .milliseconds(5))
      }
      guard let image = image(from: source, maxPixels: maxLongEdge),
            let pool = adaptor.pixelBufferPool,
            let buffer = draw(image, in: size, from: pool),
            adaptor.append(buffer, withPresentationTime: CMTime(seconds: Double(index) * hold, preferredTimescale: 600))
      else { throw writer.error ?? StillsError.unreadable }
    }
    input.markAsFinished()
    let duration = Double(sources.count) * hold
    writer.endSession(atSourceTime: CMTime(seconds: duration, preferredTimescale: 600))
    await writer.finishWriting()
    guard writer.status == .completed else { throw writer.error ?? StillsError.unreadable }
    done = true
    return Movie(duration: duration, size: size)
  }

  /// An image's upright size in pixels, without decoding it.
  static func pixelSize(_ source: CGImageSource) -> CGSize? {
    guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { return nil }
    // EXIF orientations 5–8 are turned a quarter.
    let turned = ((properties[kCGImagePropertyOrientation] as? Int) ?? 1) >= 5
    return turned ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
  }

  /// Decodes an image to at most `maxPixels` on its long edge, upright.
  static func image(from source: CGImageSource, maxPixels: Int) -> CGImage? {
    CGImageSourceCreateThumbnailAtIndex(source, 0, [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceThumbnailMaxPixelSize: maxPixels,
      kCGImageSourceShouldCacheImmediately: true,
    ] as CFDictionary)
  }

  /// `image` fitted (centered, on black) into a buffer of `size`.
  private static func draw(_ image: CGImage, in size: CGSize, from pool: CVPixelBufferPool) -> CVPixelBuffer? {
    var buffer: CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer) == kCVReturnSuccess, let buffer else { return nil }
    CVPixelBufferLockBaseAddress(buffer, [])
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
    guard let space = CGColorSpace(name: CGColorSpace.sRGB),
          let context = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer),
            width: Int(size.width),
            height: Int(size.height),
            bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
            space: space,
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
          )
    else { return nil }
    context.setFillColor(gray: 0, alpha: 1)
    context.fill(CGRect(origin: .zero, size: size))
    let scale = min(size.width / CGFloat(image.width), size.height / CGFloat(image.height))
    let fitted = CGSize(width: CGFloat(image.width) * scale, height: CGFloat(image.height) * scale)
    context.interpolationQuality = .high
    context.draw(image, in: CGRect(
      x: (size.width - fitted.width) / 2,
      y: (size.height - fitted.height) / 2,
      width: fitted.width,
      height: fitted.height
    ))
    return buffer
  }
}

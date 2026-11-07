import AVFoundation
import CoreVideo
import Darwin
import Foundation

struct WriterFixtureError: Error, CustomStringConvertible {
  let description: String
}

func require(_ value: Bool, _ message: String) throws {
  if !value { throw WriterFixtureError(description: message) }
}

func makeConfiguration() -> VideoEncodingConfiguration {
  VideoEncodingConfiguration(
    codec: .h264,
    width: 64,
    height: 64,
    averageBitRate: 500_000,
    maxKeyFrameIntervalDuration: 1,
    expectedSourceFrameRate: 30,
    allowsFrameReordering: true,
    expectsMediaDataInRealTime: false,
    profileLevel: AVVideoProfileLevelH264HighAutoLevel,
    colorProperties: .rec709,
    pixelBufferPool: VideoPixelBufferPool(
      pixelFormat: kCVPixelFormatType_32BGRA,
      usesIOSurface: true
    )
  )
}

func makeFrame(index: Int, origin: CMTime) throws -> (CVPixelBuffer, CMTime) {
  var pixelBuffer: CVPixelBuffer?
  let attributes: [String: Any] = [
    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
    kCVPixelBufferWidthKey as String: 64,
    kCVPixelBufferHeightKey as String: 64,
    kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any](),
  ]
  guard CVPixelBufferCreate(
    kCFAllocatorDefault, 64, 64, kCVPixelFormatType_32BGRA,
    attributes as CFDictionary, &pixelBuffer
  ) == kCVReturnSuccess, let pixelBuffer else {
    throw WriterFixtureError(description: "could not create synthetic BGRA frame")
  }
  CVBufferSetAttachment(pixelBuffer, kCVImageBufferColorPrimariesKey,
                        kCVImageBufferColorPrimaries_ITU_R_709_2, .shouldPropagate)
  CVBufferSetAttachment(pixelBuffer, kCVImageBufferTransferFunctionKey,
                        kCVImageBufferTransferFunction_ITU_R_709_2, .shouldPropagate)
  CVBufferSetAttachment(pixelBuffer, kCVImageBufferYCbCrMatrixKey,
                        kCVImageBufferYCbCrMatrix_ITU_R_709_2, .shouldPropagate)
  guard CVPixelBufferLockBaseAddress(pixelBuffer, []) == kCVReturnSuccess,
        let base = CVPixelBufferGetBaseAddress(pixelBuffer) else {
    throw WriterFixtureError(description: "could not lock synthetic BGRA frame")
  }
  let colors: [(UInt8, UInt8, UInt8)] = [
    (0, 0, 235), (0, 220, 0), (235, 0, 0),
  ]
  let (blue, green, red) = colors[min(index / 10, colors.count - 1)]
  let bytes = base.assumingMemoryBound(to: UInt8.self)
  let rowBytes = CVPixelBufferGetBytesPerRow(pixelBuffer)
  for y in 0..<64 {
    for x in 0..<64 {
      let offset = y * rowBytes + x * 4
      bytes[offset] = blue
      bytes[offset + 1] = green
      bytes[offset + 2] = red
      bytes[offset + 3] = 255
    }
  }
  CVPixelBufferUnlockBaseAddress(pixelBuffer, [])
  return (pixelBuffer, CMTimeAdd(origin, CMTime(value: Int64(index), timescale: 30)))
}

func writeAll(_ fd: Int32, data: Data) throws {
  try data.withUnsafeBytes { bytes in
    guard let base = bytes.baseAddress else {
      throw WriterFixtureError(description: "segment data had no byte storage")
    }
    var offset = 0
    while offset < bytes.count {
      let count = Darwin.write(fd, base.advanced(by: offset), bytes.count - offset)
      if count < 0 && errno == EINTR { continue }
      guard count > 0 else {
        throw WriterFixtureError(description: "segment sink write failed: errno \(errno)")
      }
      offset += count
    }
  }
}

func readBack(_ url: URL) async throws -> (samples: Int, first: CMTime, colors: [(UInt8, UInt8, UInt8)]) {
  let asset = AVURLAsset(url: url)
  guard let track = try await asset.loadTracks(withMediaType: .video).first else {
    throw WriterFixtureError(description: "fragment stream had no readable video track")
  }
  let reader = try AVAssetReader(asset: asset)
  let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
  ])
  output.alwaysCopiesSampleData = false
  guard reader.canAdd(output) else {
    throw WriterFixtureError(description: "AVAssetReader refused BGRA output")
  }
  reader.add(output)
  guard reader.startReading() else {
    throw WriterFixtureError(description: reader.error?.localizedDescription ?? "reader failed to start")
  }

  var count = 0
  var first = CMTime.invalid
  var previous = CMTime.invalid
  var sampledColors: [(UInt8, UInt8, UInt8)] = []
  while let sample = output.copyNextSampleBuffer() {
    let pts = CMSampleBufferGetPresentationTimeStamp(sample)
    guard pts.isNumeric else { throw WriterFixtureError(description: "decoded PTS was not numeric") }
    let expectedPTS = CMTime(value: Int64(count), timescale: 30)
    guard CMTimeCompare(pts, expectedPTS) == 0 else {
      throw WriterFixtureError(description: "decoded PTS did not preserve source interval at sample \(count)")
    }
    if count == 0 {
      first = pts
    } else if CMTimeCompare(pts, previous) <= 0 {
      throw WriterFixtureError(description: "decoded PTS order was not strictly increasing")
    }
    previous = pts
    guard let pixels = CMSampleBufferGetImageBuffer(sample),
          CVPixelBufferGetWidth(pixels) == 64, CVPixelBufferGetHeight(pixels) == 64,
          CVPixelBufferGetPixelFormatType(pixels) == kCVPixelFormatType_32BGRA,
          CVPixelBufferLockBaseAddress(pixels, .readOnly) == kCVReturnSuccess,
          let base = CVPixelBufferGetBaseAddress(pixels) else {
      throw WriterFixtureError(description: "decoded frame dimensions/layout did not match")
    }
    if count == 2 || count == 15 || count == 27 {
      let offset = 32 * CVPixelBufferGetBytesPerRow(pixels) + 32 * 4
      let p = base.advanced(by: offset).assumingMemoryBound(to: UInt8.self)
      sampledColors.append((p[2], p[1], p[0]))
    }
    CVPixelBufferUnlockBaseAddress(pixels, .readOnly)
    count += 1
  }
  guard reader.status == .completed else {
    throw WriterFixtureError(description: reader.error?.localizedDescription ?? "reader did not complete")
  }
  return (count, first, sampledColors)
}

func createOwnedOutput(_ path: String) throws -> Int32 {
  let fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW, S_IRUSR | S_IWUSR)
  guard fd >= 0 else { throw WriterFixtureError(description: "exclusive output open failed: errno \(errno)") }
  var metadata = stat()
  guard fstat(fd, &metadata) == 0,
        (metadata.st_mode & S_IFMT) == S_IFREG,
        (metadata.st_mode & 0o777) == 0o600,
        metadata.st_nlink == 1 else {
    close(fd)
    throw WriterFixtureError(description: "test output was not an owned regular 0600 single-link file")
  }
  return fd
}

func appendFrames(to writer: SegmentedVideoWriter, origin: CMTime, count: Int) async throws {
  for index in 0..<count {
    let (pixels, pts) = try makeFrame(index: index, origin: origin)
    let deadline = ProcessInfo.processInfo.systemUptime + 5
    while try !writer.append(pixels, at: pts) {
      guard ProcessInfo.processInfo.systemUptime < deadline else {
        throw WriterFixtureError(description: "synthetic writer readiness exceeded five seconds")
      }
      try await Task.sleep(nanoseconds: 1_000_000)
    }
  }
}

func runSuccessfulWriter(directory: URL, retain: Bool) async throws -> String {
  let path = directory.appendingPathComponent("segmented.mp4", isDirectory: false).path
  var fd = try createOwnedOutput(path)
  var keepOutput = false
  defer {
    if fd >= 0 { _ = close(fd) }
    if !keepOutput { unlink(path) }
  }
  let origin = CMTime(value: 90, timescale: 30)
  let writer = try SegmentedVideoWriter(
    configuration: makeConfiguration(),
    timeOrigin: origin,
    preferredSegmentInterval: CMTime(value: 10, timescale: 30),
    maximumBytes: 8 * 1024 * 1024,
    maximumSegments: 32
  ) { data, _ in try writeAll(fd, data: data) }
  try await appendFrames(to: writer, origin: origin, count: 30)
  let completion = try await writer.finish()
  try require(completion.segments >= 2, "writer did not emit initialization and media segments")
  guard fsync(fd) == 0 else { throw WriterFixtureError(description: "output fsync failed: errno \(errno)") }
  var metadata = stat()
  var namedMetadata = stat()
  guard fstat(fd, &metadata) == 0,
        lstat(path, &namedMetadata) == 0,
        metadata.st_dev == namedMetadata.st_dev,
        metadata.st_ino == namedMetadata.st_ino,
        metadata.st_size == off_t(completion.bytes),
        (metadata.st_mode & 0o777) == 0o600,
        metadata.st_nlink == 1 else {
    throw WriterFixtureError(description: "sink bytes or output identity did not match completion")
  }
  let closeStatus = close(fd)
  fd = -1
  guard closeStatus == 0 else { throw WriterFixtureError(description: "output close failed: errno \(errno)") }
  let readback = try await readBack(URL(fileURLWithPath: path))
  try require(completion.appendedSamples == 30, "writer sample count changed")
  try require(readback.samples == 30, "reader did not return all 30 frames")
  try require(CMTimeCompare(readback.first, .zero) == 0, "encoded first PTS did not match explicit origin")
  try require(readback.colors.count == 3, "missing color transition samples")
  try require(readback.colors[0].0 > 140 && readback.colors[0].1 < 100 && readback.colors[0].2 < 100,
              "first decoded color was not red")
  try require(readback.colors[1].1 > 140 && readback.colors[1].0 < 100 && readback.colors[1].2 < 100,
              "middle decoded color was not green")
  try require(readback.colors[2].2 > 140 && readback.colors[2].0 < 100 && readback.colors[2].1 < 100,
              "last decoded color was not blue")
  keepOutput = retain
  return "samples=\(readback.samples) first_pts=\(readback.first.value)/\(readback.first.timescale) segments=\(completion.segments) bytes=\(completion.bytes) colors=RGB retained_path=\(retain ? path : "none")"
}

func runSinkFailure(byteLimit: Int) async throws {
  let origin = CMTime(value: 0, timescale: 30)
  var sinkCalls = 0
  let writer = try SegmentedVideoWriter(
    configuration: makeConfiguration(),
    timeOrigin: origin,
    preferredSegmentInterval: CMTime(value: 10, timescale: 30),
    maximumBytes: byteLimit,
    maximumSegments: 32
  ) { _, _ in
    sinkCalls += 1
    throw WriterFixtureError(description: "fixture sink refused bytes")
  }
  var appendFailure: SegmentedVideoWriterError?
  do {
    try await appendFrames(to: writer, origin: origin, count: 30)
  } catch let error as SegmentedVideoWriterError {
    appendFailure = error
  }
  // Always finish, including when a callback refused the sink during append. This cancels
  // incomplete native writing and must preserve the original failure instead of partial success.
  do {
    _ = try await writer.finish()
    throw WriterFixtureError(description: "failed sink was reported as finalized")
  } catch let error as SegmentedVideoWriterError {
    if let appendFailure {
      try require(error == appendFailure, "finish changed the earlier sink failure")
    }
    switch error {
    case .sinkFailed(_) where byteLimit > 1:
      try require(sinkCalls > 0, "sink failure callback was not exercised")
    case .byteLimitExceeded where byteLimit == 1:
      try require(sinkCalls == 0, "sink ran after byte limit refusal")
    default:
      throw WriterFixtureError(description: "unexpected writer failure: \(error)")
    }
  }
}

@main struct SegmentedWriterTests {
  static func main() async throws {
    guard CommandLine.arguments.count == 2 || CommandLine.arguments.count == 3,
          CommandLine.arguments.count == 2 || CommandLine.arguments[2] == "--retain" else {
      throw WriterFixtureError(description: "usage: segmented-writer-tests PRIVATE_DIRECTORY [--retain]")
    }
    let directory = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
    var directoryMetadata = stat()
    guard lstat(directory.path, &directoryMetadata) == 0,
          (directoryMetadata.st_mode & S_IFMT) == S_IFDIR,
          directoryMetadata.st_uid == getuid(),
          (directoryMetadata.st_mode & 0o777) == 0o700 else {
      throw WriterFixtureError(description: "output directory must be an owned private 0700 directory")
    }
    try await runSinkFailure(byteLimit: 8 * 1024 * 1024)
    try await runSinkFailure(byteLimit: 1)
    let result = try await runSuccessfulWriter(directory: directory, retain: CommandLine.arguments.count == 3)
    print("SEGMENTED_WRITER_PASS \(result) sink_failure=refused byte_limit=refused partial_success=false")
  }
}

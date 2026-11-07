// File-only AVAssetReader discriminator for the ignored Core SCK clock proof.
// Input pixels are copied from the exact accepted CMSampleBuffer callback and bound to its
// source/host PTS. The movie is read sample-by-sample; no helper encoder or alternate recorder.
import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation

struct ProofError: Error, CustomStringConvertible {
  let description: String
  init(_ description: String) { self.description = description }
}

struct Timestamp: Decodable {
  let value: Int64
  let timescale: Int32
  let flags: UInt32
  let epoch: Int64
  var seconds: Double { Double(value) / Double(timescale) }
  var valid: Bool { timescale > 0 && (flags & 0x1d) == 1 && seconds.isFinite }
}

struct SourceSample: Decodable {
  let sourcePts: Timestamp
  let hostPts: Timestamp
  let captureRelativeMs: UInt64
  let width: Int
  let height: Int
  let pixelFormat: String
  let planar: Bool
  let bytesPerRow: Int
  let dataSize: Int
  let byteOffset: Int
  let byteLength: Int
  let colorAttachments: ColorAttachments?
}

struct ColorAttachments: Decodable {
  let primaries: String?
  let transferFunction: String?
  let ycbcrMatrix: String?
  let cgColorSpace: String?
}

struct SourceDocument: Decodable {
  let schema: String
  let admission: Timestamp
  let deliberateSetupDelayMs: Int
  let sourceSpanMs: UInt64
  let pixelBytesFile: String
  let sourceColorSpaceProfileStatus: String?
  let sourceColorSpaceICC: ICCProfile?
  let samples: [SourceSample]
  let encodedOrigin: String
}

struct ICCProfile: Decodable {
  let encoding: String
  let byteLength: Int
  let data: String
}

struct Match {
  let observation: Int
  let moviePts: Double
  let score: Double
}

struct BestPixelMatch {
  var score = Double.infinity
  var pts = 0.0
  var frame = -1
  var supported = 0
}

let diagnosticSRGBSpace = CGColorSpace(name: CGColorSpace.sRGB)!

func fail(_ message: String) -> Never {
  FileHandle.standardError.write(Data((message + "\n").utf8))
  exit(1)
}

func meanAbsoluteDifference(
  sourceData: Data, sourceOffset: Int, sourceStride: Int,
  movie: UnsafeRawPointer, movieStride: Int,
  width: Int, height: Int
) -> Double {
  sourceData.withUnsafeBytes { raw in
    guard let source = raw.baseAddress?.advanced(by: sourceOffset) else { return .infinity }
    var total: UInt64 = 0
    var count: UInt64 = 0
    // Four-pixel grid samples retain the synthetic scene's large unique transitions while
    // keeping this diagnostic bounded on 640x360 native buffers.
    for y in stride(from: 0, to: height, by: 4) {
      let a = source.advanced(by: y * sourceStride).assumingMemoryBound(to: UInt8.self)
      let b = movie.advanced(by: y * movieStride).assumingMemoryBound(to: UInt8.self)
      for x in stride(from: 0, to: width, by: 4) {
        let at = x * 4
        for channel in 0..<3 {
          total += UInt64(abs(Int(a[at + channel]) - Int(b[at + channel])))
          count += 1
        }
      }
    }
    return count == 0 ? .infinity : Double(total) / Double(count)
  }
}

func sourceRoot(from log: String) throws -> URL {
  let roots = log.split(separator: "\n").compactMap { line -> String? in
    let marker = "SCK_FIXTURE_RETAINED="
    guard let range = line.range(of: marker) else { return nil }
    return String(line[range.upperBound...])
  }
  guard let root = roots.last, !root.isEmpty else { throw ProofError("retained fixture root missing from SCK log") }
  return URL(fileURLWithPath: root, isDirectory: true)
}

func mediaStyleSRGBPixels(_ image: CGImage) -> Data? {
  let width = image.width, height = image.height
  var pixels = Data(count: width * height * 4)
  let drawn = pixels.withUnsafeMutableBytes { bytes -> Bool in
    guard let bitmap = CGContext(
      data: bytes.baseAddress, width: width, height: height, bitsPerComponent: 8,
      bytesPerRow: width * 4, space: diagnosticSRGBSpace,
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return false }
    bitmap.interpolationQuality = .high
    bitmap.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    return true
  }
  return drawn ? pixels : nil
}

func colorSpace(from profile: ICCProfile?) -> CGColorSpace? {
  guard let profile, profile.encoding == "base64", (1...32 * 1024).contains(profile.byteLength),
        let data = Data(base64Encoded: profile.data), data.count == profile.byteLength,
        let space = CGColorSpace(iccData: data as CFData),
        space.model == .rgb, space.numberOfComponents == 3 else { return nil }
  return space
}

func cgImageFromBGRA(_ bytes: Data, width: Int, height: Int, stride: Int, space: CGColorSpace) -> CGImage? {
  let (requiredBytes, overflow) = stride.multipliedReportingOverflow(by: height)
  guard width > 0, height > 0, !overflow, stride >= width * 4,
        bytes.count >= requiredBytes,
        let provider = CGDataProvider(data: bytes as CFData) else { return nil }
  let bitmap = CGBitmapInfo.byteOrder32Little.union(
    CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue)
  )
  return CGImage(
    width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
    bytesPerRow: stride, space: space, bitmapInfo: bitmap, provider: provider,
    decode: nil, shouldInterpolate: false, intent: .defaultIntent
  )
}

func decodedColorSpace(_ pixel: CVPixelBuffer) -> CGColorSpace? {
  let propagated = CVBufferCopyAttachments(pixel, .shouldPropagate) as NSDictionary?
  let local = CVBufferCopyAttachments(pixel, .shouldNotPropagate) as NSDictionary?
  guard let attachment = propagated?[kCVImageBufferCGColorSpaceKey as String]
    ?? local?[kCVImageBufferCGColorSpaceKey as String] else { return nil }
  let value = attachment as CFTypeRef
  guard CFGetTypeID(value) == CGColorSpace.typeID else { return nil }
  let space = unsafeBitCast(value, to: CGColorSpace.self)
  guard space.model == .rgb, space.numberOfComponents == 3 else { return nil }
  return space
}

func colorAttachments(_ pixel: CVPixelBuffer) -> ColorAttachments {
  let propagated = CVBufferCopyAttachments(pixel, .shouldPropagate) as NSDictionary?
  let local = CVBufferCopyAttachments(pixel, .shouldNotPropagate) as NSDictionary?
  func read(_ key: CFString) -> String? {
    (propagated?[key as String] ?? local?[key as String]).map { String(describing: $0) }
  }
  return ColorAttachments(
    primaries: read(kCVImageBufferColorPrimariesKey),
    transferFunction: read(kCVImageBufferTransferFunctionKey),
    ycbcrMatrix: read(kCVImageBufferYCbCrMatrixKey),
    cgColorSpace: read(kCVImageBufferCGColorSpaceKey)
  )
}

func colorAttachmentSummary(_ color: ColorAttachments) -> String {
  [
    "primaries=\(color.primaries ?? "missing")",
    "transfer=\(color.transferFunction ?? "missing")",
    "matrix=\(color.ycbcrMatrix ?? "missing")",
    "cgColorSpace=\(color.cgColorSpace ?? "missing")",
  ].joined(separator: ",")
}

func formatColorSummary(_ description: CMFormatDescription?) -> String {
  guard let description,
        let extensions = CMFormatDescriptionGetExtensions(description) as NSDictionary? else {
    return "unavailable"
  }
  let fields: [(String, CFString)] = [
    ("primaries", kCMFormatDescriptionExtension_ColorPrimaries),
    ("transfer", kCMFormatDescriptionExtension_TransferFunction),
    ("matrix", kCMFormatDescriptionExtension_YCbCrMatrix),
  ]
  return fields.map { name, key in
    "\(name)=\(extensions[key as String].map(String.init(describing:)) ?? "missing")"
  }.joined(separator: ",")
}

func readback(root: URL, strictCorrespondence: Bool) async throws {
  let documentURL = root.appendingPathComponent("sck-source-samples.json")
  let documentData = try Data(contentsOf: documentURL)
  guard documentData.count <= 128 * 1024 else { throw ProofError("source observation metadata exceeds limit") }
  let document = try JSONDecoder().decode(SourceDocument.self, from: documentData)
  guard [
    "apparent.test-sck-source-samples/1",
    "apparent.test-sck-source-samples/2",
    "apparent.test-sck-source-samples/3",
  ].contains(document.schema),
        document.encodedOrigin.hasPrefix("unproven"),
        document.pixelBytesFile == "sck-source-pixels.bgra",
        (8...12).contains(document.samples.count),
        document.sourceSpanMs >= 500,
        document.deliberateSetupDelayMs >= 200,
        document.admission.valid else { throw ProofError("source observation binding or bounds invalid") }
  let pixelURL = root.appendingPathComponent(document.pixelBytesFile)
  let pixelData = try Data(contentsOf: pixelURL, options: .mappedIfSafe)
  guard pixelData.count <= 24 * 1024 * 1024 else { throw ProofError("source pixel evidence exceeds limit") }
  var expectedByteOffset = 0
  for sample in document.samples {
    guard sample.sourcePts.valid, sample.hostPts.valid,
          sample.hostPts.epoch == document.admission.epoch,
          sample.pixelFormat == "42475241", !sample.planar,
          sample.width == 640, sample.height == 360,
          sample.bytesPerRow >= sample.width * 4,
          sample.height > 0,
          sample.bytesPerRow.multipliedReportingOverflow(by: sample.height).overflow == false,
          sample.dataSize >= sample.bytesPerRow * sample.height,
          sample.byteLength == sample.bytesPerRow * sample.height,
          sample.byteOffset == expectedByteOffset,
          sample.byteOffset >= 0,
          sample.byteOffset <= pixelData.count,
          sample.byteLength <= pixelData.count - sample.byteOffset else {
      throw ProofError("unsupported or malformed source pixel observation")
    }
    expectedByteOffset += sample.byteLength
  }
  guard expectedByteOffset == pixelData.count else { throw ProofError("source pixel byte inventory mismatch") }

  let movieURL = root.appendingPathComponent("sck-store-stage/guarded-output.mp4")
  let asset = AVURLAsset(url: movieURL, options: [AVURLAssetPreferPreciseDurationAndTimingKey: true])
  let tracks = try await asset.loadTracks(withMediaType: .video)
  guard let track = tracks.first else { throw ProofError("finalized movie has no video track") }
  let reader = try AVAssetReader(asset: asset)
  let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
  ])
  output.alwaysCopiesSampleData = false
  guard reader.canAdd(output) else { throw ProofError("AVAssetReader rejected BGRA video output") }
  reader.add(output)
  guard reader.startReading() else { throw ProofError(reader.error?.localizedDescription ?? "AVAssetReader start failed") }

  var rawBest = Array(repeating: BestPixelMatch(), count: document.samples.count)
  var srgbBest = Array(repeating: BestPixelMatch(), count: document.samples.count)
  let sourceProfile = colorSpace(from: document.sourceColorSpaceICC)
  let sourceProfileIsBound = document.sourceColorSpaceProfileStatus == "shared-identical-icc-profile"
  print("SCK_SOURCE_COLOR_PROFILE status=\(document.sourceColorSpaceProfileStatus ?? "missing-schema-field") icc_bytes=\(document.sourceColorSpaceICC?.byteLength ?? 0) decoded=\(sourceProfile == nil ? "unavailable" : "validated-icc")")
  let canonicalSources: [Data?] = document.samples.map { sample in
    guard sourceProfileIsBound, let sourceProfile,
          sample.byteOffset >= 0, sample.byteLength > 0,
          sample.byteOffset <= pixelData.count,
          sample.byteLength <= pixelData.count - sample.byteOffset else { return nil }
    let bytes = pixelData.subdata(in: sample.byteOffset..<(sample.byteOffset + sample.byteLength))
    guard let image = cgImageFromBGRA(
      bytes, width: sample.width, height: sample.height,
      stride: sample.bytesPerRow, space: sourceProfile
    ) else { return nil }
    return mediaStyleSRGBPixels(image)
  }
  var moviePts: [Double] = []
  var firstEncodedPts: CMTime?
  var firstDecodedColorSpace: CGColorSpace?
  var decodedProfileStable = true
  var canonicalMatches: [Match] = []
  while let buffer = output.copyNextSampleBuffer() {
    guard moviePts.count < 2_000 else { throw ProofError("decoded frame count exceeds limit") }
    let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
    let seconds = CMTimeGetSeconds(pts)
    guard pts.isValid, pts.isNumeric, seconds.isFinite else { throw ProofError("movie sample has invalid PTS") }
    if firstEncodedPts == nil { firstEncodedPts = pts }
    moviePts.append(seconds)
    guard let pixel = CMSampleBufferGetImageBuffer(buffer) else { throw ProofError("movie sample has no pixel buffer") }
    let decodedAttachments = colorAttachments(pixel)
    let decodedColor = colorAttachmentSummary(decodedAttachments)
    if moviePts.count == 1 {
      let formatColor = formatColorSummary(CMSampleBufferGetFormatDescription(buffer))
      print([
        "SCK_DECODED_BUFFER_LAYOUT",
        "format=\(CVPixelBufferGetPixelFormatType(pixel))",
        "planar=\(CVPixelBufferIsPlanar(pixel))",
        "width=\(CVPixelBufferGetWidth(pixel))",
        "height=\(CVPixelBufferGetHeight(pixel))",
        "row_bytes=\(CVPixelBufferGetBytesPerRow(pixel))",
        "data_size=\(CVPixelBufferGetDataSize(pixel))",
        "color={\(decodedColor)}",
        "track_color={\(formatColor)}",
      ].joined(separator: " "))
    }
    guard CVPixelBufferGetPixelFormatType(pixel) == kCVPixelFormatType_32BGRA,
          CVPixelBufferGetWidth(pixel) == 640, CVPixelBufferGetHeight(pixel) == 360 else {
      throw ProofError("AVAssetReader returned unsupported movie pixel layout")
    }
    guard CVPixelBufferLockBaseAddress(pixel, .readOnly) == kCVReturnSuccess else {
      throw ProofError("could not lock decoded movie pixels")
    }
    defer { CVPixelBufferUnlockBaseAddress(pixel, .readOnly) }
    guard let movieBase = CVPixelBufferGetBaseAddress(pixel) else { throw ProofError("decoded frame has no base address") }
    let movieStride = CVPixelBufferGetBytesPerRow(pixel)
    let (movieByteCount, movieSizeOverflow) = movieStride.multipliedReportingOverflow(
      by: CVPixelBufferGetHeight(pixel)
    )
    guard !movieSizeOverflow, movieByteCount > 0, movieByteCount <= 2 * 1024 * 1024 else {
      throw ProofError("decoded movie frame exceeds pixel evidence bound")
    }
    let movieBytes = Data(bytes: movieBase, count: movieByteCount)
    let frameColorSpace = decodedColorSpace(pixel)
    if let frameColorSpace {
      if let firstDecodedColorSpace {
        decodedProfileStable = decodedProfileStable
          && CFEqual(firstDecodedColorSpace, frameColorSpace)
      } else {
        firstDecodedColorSpace = frameColorSpace
      }
    } else {
      decodedProfileStable = false
    }
    let canonicalMovie = frameColorSpace.flatMap { space -> Data? in
      guard let image = cgImageFromBGRA(
        movieBytes, width: CVPixelBufferGetWidth(pixel),
        height: CVPixelBufferGetHeight(pixel), stride: movieStride, space: space
      ) else { return nil }
      return mediaStyleSRGBPixels(image)
    }
    // Decoded attachment values are taken from this exact AVAssetReader pixel buffer. The
    // summary is retained even when absent so the file-only mode identifies the missing field.
    if moviePts.count == 1 { print("SCK_DECODED_BUFFER_COLOR attachments=\(decodedColor)") }
    for (index, sample) in document.samples.enumerated() {
      let score = meanAbsoluteDifference(
        sourceData: pixelData, sourceOffset: sample.byteOffset, sourceStride: sample.bytesPerRow,
        movie: movieBase, movieStride: movieStride,
        width: sample.width, height: sample.height
      )
      if score < rawBest[index].score {
        rawBest[index].score = score
        rawBest[index].pts = seconds
        rawBest[index].frame = moviePts.count - 1
      }
      if score <= 8.0 {
        rawBest[index].supported += 1
      }
      if let canonicalSource = canonicalSources[index], let canonicalMovie {
        let canonicalScore = canonicalMovie.withUnsafeBytes { raw -> Double in
          guard let base = raw.baseAddress else { return .infinity }
          return meanAbsoluteDifference(
            sourceData: canonicalSource, sourceOffset: 0, sourceStride: sample.width * 4,
            movie: base, movieStride: sample.width * 4, width: sample.width, height: sample.height
          )
        }
        if canonicalScore < srgbBest[index].score {
          srgbBest[index].score = canonicalScore
          srgbBest[index].pts = seconds
          srgbBest[index].frame = moviePts.count - 1
        }
        if canonicalScore <= 8.0 {
          srgbBest[index].supported += 1
          canonicalMatches.append(Match(observation: index, moviePts: seconds, score: canonicalScore))
        }
      }
    }
  }
  guard reader.status == .completed, let firstEncodedPts, moviePts.count >= 8,
        let firstPts = moviePts.first else {
    throw ProofError(reader.error?.localizedDescription ?? "movie readback incomplete")
  }
  let frameIntervals = zip(moviePts.dropFirst(), moviePts).map { pair in pair.0 - pair.1 }
    .filter { $0 > 0 && $0.isFinite }.sorted()
  guard !frameIntervals.isEmpty else { throw ProofError("movie PTS has no positive frame interval") }
  let tolerance = frameIntervals[frameIntervals.count / 2]
  print([
    "SCK_MOVIE_READBACK",
    "firstEncodedPTS=\(firstEncodedPts.value)/\(firstEncodedPts.timescale)",
    "seconds=\(CMTimeGetSeconds(firstEncodedPts))",
    "frames=\(moviePts.count)",
    "observedFrameToleranceMs=\(tolerance * 1000)",
  ].joined(separator: " "))
  for index in document.samples.indices {
    let sample = document.samples[index]
    let sourceColor = sample.colorAttachments.map(colorAttachmentSummary)
      ?? "missing-entire-colorAttachments-object"
    let normalized = canonicalSources[index].map { _ in
      "best_mae=\(srgbBest[index].score),frame=\(srgbBest[index].frame),pts=\(srgbBest[index].pts),matches_le8=\(srgbBest[index].supported)"
    } ?? "unavailable-source-ICC-profile-or-CoreGraphics-image"
    let rawOffset = sample.hostPts.seconds - document.admission.seconds - (rawBest[index].pts - firstPts)
    let normalizedOffset = canonicalSources[index].map { _ in
      String((sample.hostPts.seconds - document.admission.seconds - (srgbBest[index].pts - firstPts)) * 1000)
    } ?? "unavailable"
    let colorAssessment = canonicalSources[index] == nil
      ? "unknown-source-ICC-profile-unavailable"
      : (srgbBest[index].score <= 8
        ? "CoreGraphics-source-and-decoded-color-spaces-to-sRGB-match"
        : "CoreGraphics-source-and-decoded-color-spaces-to-sRGB-no-match")
    print([
      "SCK_PIXEL_DIAGNOSTIC",
      "sample=\(index)",
      "source_pts=\(sample.sourcePts.seconds)",
      "raw_best_mae=\(rawBest[index].score)",
      "raw_best_frame=\(rawBest[index].frame)",
      "raw_best_pts=\(rawBest[index].pts)",
      "raw_best_offset_ms=\(rawOffset * 1000)",
      "raw_matches_le8=\(rawBest[index].supported)",
      "source_color={\(sourceColor)}",
      "cg_srgb=\(normalized)",
      "cg_srgb_best_offset_ms=\(normalizedOffset)",
      "color_assessment=\(colorAssessment)",
    ].joined(separator: " "))
  }
  print([
    "SCK_CANONICAL_PROFILE_STATUS",
    "source=\(document.sourceColorSpaceProfileStatus ?? "missing-schema-field")",
    "sourceSamplesConverted=\(canonicalSources.compactMap { $0 }.count)/\(canonicalSources.count)",
    "decodedProfilePresent=\(firstDecodedColorSpace != nil)",
    "decodedProfileStable=\(decodedProfileStable)",
    "canonicalMatchesLe8=\(canonicalMatches.count)",
  ].joined(separator: " "))
  guard strictCorrespondence else {
    print("SCK_FILE_ONLY_DIAGNOSTIC frames=\(moviePts.count) sourceSamples=\(document.samples.count) encoded_origin=unproven")
    return
  }
  guard sourceProfileIsBound, sourceProfile != nil,
        canonicalSources.allSatisfy({ $0 != nil }) else {
    throw ProofError("source ICC profile is missing, varying, malformed, or unsupported")
  }
  guard decodedProfileStable, firstDecodedColorSpace != nil,
        canonicalMatches.count > 0 else {
    throw ProofError("decoded AVAssetReader CGColorSpace is missing, varying, or unsupported")
  }
  let originMatches = canonicalMatches
  let admissionSeconds = document.admission.seconds
  let offsets = originMatches.map { match in
    document.samples[match.observation].hostPts.seconds - admissionSeconds - (match.moviePts - firstPts)
  }.filter { $0.isFinite && $0 >= -tolerance && $0 * 1000 <= 9_007_199_254_740_991 }
  guard !offsets.isEmpty else { throw ProofError("no source sample matched a decoded movie frame") }

  // A valid origin must align every retained source observation within one observed frame
  // interval. Collapse offsets only within that tolerance; distinct supported clusters remain
  // ambiguous (for example, repeated-looking scene phases) and are not accepted.
  var supported: [Double] = []
  for seed in offsets {
    var perObservation: [Double] = []
    var allCovered = true
    for index in document.samples.indices {
      let candidates = originMatches.filter { $0.observation == index }.map { match in
        document.samples[index].hostPts.seconds - admissionSeconds - (match.moviePts - firstPts)
      }
      guard let nearest = candidates.min(by: { abs($0 - seed) < abs($1 - seed) }),
            abs(nearest - seed) <= tolerance else {
        allCovered = false
        break
      }
      perObservation.append(nearest)
    }
    if allCovered,
       let low = perObservation.min(), let high = perObservation.max(),
       high - low <= tolerance {
      supported.append(perObservation.sorted()[perObservation.count / 2])
    }
  }
  var centers: [Double] = []
  var cluster: [Double] = []
  for value in supported.sorted() {
    if let first = cluster.first, value - first > tolerance {
      centers.append(cluster.sorted()[cluster.count / 2])
      cluster = [value]
    } else {
      cluster.append(value)
    }
  }
  if !cluster.isEmpty { centers.append(cluster.sorted()[cluster.count / 2]) }
  guard centers.count == 1 else {
    let covered = rawBest.filter { $0.supported > 0 }.count
    print([
      "SCK_CORRESPONDENCE_REJECTION",
      "reason=\(centers.isEmpty ? "no-unique-origin-cluster" : "multiple-origin-clusters")",
      "sourceSamplesWithRawPixelMatches=\(covered)/\(rawBest.count)",
      "supportedOriginClusters=\(centers.count)",
      "rawAcceptanceMaeLimit=8.0",
      "toleranceMs=\(tolerance * 1000)",
      "encodedOrigin=unproven",
    ].joined(separator: " "))
    throw ProofError("source-to-encoded correspondence is unmatched or ambiguous (supported origin clusters: \(centers.count))")
  }
  let offsetMs = (centers[0] * 1000).rounded()
  guard offsetMs >= 0, offsetMs <= 9_007_199_254_740_991 else { throw ProofError("derived origin outside public clock range") }
  let bestScore = originMatches.map(\.score).min() ?? .infinity
  print([
    "SCK_FRAME_CORRESPONDENCE=unique",
    "firstEncodedPTS=\(CMTimeGetSeconds(firstEncodedPts))",
    "movieFrames=\(moviePts.count)",
    "sourceSamples=\(document.samples.count)",
    "toleranceMs=\(tolerance * 1000)",
    "bestPixelMae=\(bestScore)",
    "videoOffsetMs=\(UInt64(offsetMs))",
  ].joined(separator: " "))
}

@main struct SckClockReadback {
  static func main() async {
    let arguments = CommandLine.arguments
    do {
      if arguments.count == 3, arguments[1] == "--fixture-root" {
        try await readback(root: URL(fileURLWithPath: arguments[2], isDirectory: true), strictCorrespondence: false)
      } else if arguments.count == 2 {
        let logURL = URL(fileURLWithPath: arguments[1])
        let log = try String(contentsOf: logURL, encoding: .utf8)
        try await readback(root: sourceRoot(from: log), strictCorrespondence: true)
      } else {
        fail("usage: sck-clock <sck-store-guard.log> | sck-clock --fixture-root <retained-fixture-directory>")
      }
    }
    catch { fail("SCK_FRAME_CORRESPONDENCE=unproven reason=\(error)") }
  }
}

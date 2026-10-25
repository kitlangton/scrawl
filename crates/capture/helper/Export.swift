@preconcurrency import AVFoundation
import AppKit
import CoreImage
import CoreVideo

// `scrawl-helper export PLAN.json`: renders a bundle's recording as one shareable .mp4.
//
// Rust (src/export) decides what to show and writes the plan: the crop, an optional camera
// rectangle per time, the annotations as vectors, and which audio to use. This only renders it:
//
//   AVAssetReader (hardware decode, BGRA)  →  Core Image (crop / camera rectangle, GPU)
//   →  Core Graphics (ink, laser, ripples, drawn into the same pixel buffer)
//   →  AVAssetWriter (hardware H.264 / HEVC) + AAC audio from an AVMutableComposition.
//
// Screen recordings only deliver a frame when the screen changes, so output is a constant frame
// rate: each tick shows the latest source frame, which lets the camera and the ink move while the
// screen holds still. Every time in the plan is on the video's own clock (the first frame is 0).
// Progress is `{"progress": 0…1}` lines, then `{"done": {...}}`.

struct ExportPlan: Decodable {
  var video: String
  var out: String
  var audio: String?
  var audioStartMs: Int
  var audioLengthMs: Int
  var width: Int
  var height: Int
  var fps: Int
  var codec: String
  var durationMs: Int
  var crop: [Double]
  var camera: [[Double]]?
  var ink: Ink

  struct Ink: Decodable {
    var k: Double
    var pinScale: Double
    var strokes: [Stroke]
    var pins: [Pin]
    var lasers: [Laser]
    var ripples: [Ripple]
    var trails: [Trail]
  }
  struct Stroke: Decodable {
    var snaps: [Snap]
    var fadeAt: Double
    var fadeMs: Double
  }
  struct Snap: Decodable {
    var t: Double
    var poly: [Double]
  }
  struct Pin: Decodable {
    var x: Double, y: Double
    var n: Int
    var t: Double
    var fadeAt: Double
    var fadeMs: Double
  }
  struct Laser: Decodable { var frames: [LaserFrame] }
  struct LaserFrame: Decodable {
    var t: Double
    var pts: [Double]
  }
  struct Ripple: Decodable {
    var x: Double, y: Double
    var t: Double
  }
  struct Trail: Decodable {
    var pts: [Double]
    var t: Double
    var t1: Double
  }
}

/// Maps the capture's normalized coordinates to output pixels (y down) for one camera rectangle.
struct ExportView {
  let sourceW: Double, sourceH: Double
  let originX: Double, originY: Double
  let scaleX: Double, scaleY: Double
  var scale: Double { (scaleX + scaleY) / 2 }

  init(sourceW: Double, sourceH: Double, rect: [Double], outW: Double, outH: Double) {
    self.sourceW = sourceW
    self.sourceH = sourceH
    originX = rect[0] * sourceW
    originY = rect[1] * sourceH
    scaleX = outW / max(1, (rect[2] - rect[0]) * sourceW)
    scaleY = outH / max(1, (rect[3] - rect[1]) * sourceH)
  }

  func point(_ x: Double, _ y: Double) -> CGPoint {
    CGPoint(x: (x * sourceW - originX) * scaleX, y: (y * sourceH - originY) * scaleY)
  }
}

enum ExportInk {
  static let red = (r: 1.0, g: 59.0 / 255, b: 48.0 / 255)

  static func fade(_ t: Double, at: Double, over: Double) -> Double {
    if t <= at { return 1 }
    let u = min(1, (t - at) / max(1, over))
    return 1 - u * u * (3 - 2 * u)
  }

  static func draw(_ ctx: CGContext, t: Double, view: ExportView, ink: ExportPlan.Ink) {
    let s = view.scale
    let k = ink.k
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)

    for stroke in ink.strokes {
      guard let first = stroke.snaps.first, t >= first.t else { continue }
      let alpha = fade(t, at: stroke.fadeAt, over: stroke.fadeMs)
      guard alpha > 0.002 else { continue }
      var lo = 0, hi = stroke.snaps.count - 1
      while lo < hi {
        let mid = (lo + hi + 1) / 2
        if stroke.snaps[mid].t <= t { lo = mid } else { hi = mid - 1 }
      }
      let poly = stroke.snaps[lo].poly
      guard poly.count >= 6 else { continue }
      ctx.beginPath()
      ctx.move(to: view.point(poly[0], poly[1]))
      var i = 2
      while i + 1 < poly.count {
        ctx.addLine(to: view.point(poly[i], poly[i + 1]))
        i += 2
      }
      ctx.closePath()
      ctx.setFillColor(CGColor(red: red.r, green: red.g, blue: red.b, alpha: alpha))
      ctx.fillPath()
    }

    for pin in ink.pins where t >= pin.t {
      let alpha = fade(t, at: pin.fadeAt, over: pin.fadeMs)
      guard alpha > 0.002 else { continue }
      let pop = min(1, (t - pin.t) / 160)
      let grow = 1 - pow(1 - pop, 3)
      let scale = ink.pinScale * s * (0.6 + 0.4 * grow)
      let c = view.point(pin.x, pin.y)
      let radius = 13 * scale
      func disc(_ r: Double, _ color: CGColor) {
        ctx.setFillColor(color)
        ctx.fillEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
      }
      disc(radius + 2.5 * scale, CGColor(red: 1, green: 1, blue: 1, alpha: alpha * grow))
      disc(radius, CGColor(red: red.r, green: red.g, blue: red.b, alpha: alpha * grow))
      let label = String(pin.n) as NSString
      var size = 16 * scale
      let attrs: (Double) -> [NSAttributedString.Key: Any] = {
        [.font: NSFont.systemFont(ofSize: $0, weight: .bold), .foregroundColor: NSColor(white: 1, alpha: alpha * grow)]
      }
      let width = label.size(withAttributes: attrs(size)).width
      if width > radius * 1.65 { size *= radius * 1.65 / width }
      let measured = label.size(withAttributes: attrs(size))
      NSGraphicsContext.saveGraphicsState()
      NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: true)
      label.draw(at: CGPoint(x: c.x - measured.width / 2, y: c.y - measured.height / 2), withAttributes: attrs(size))
      NSGraphicsContext.restoreGraphicsState()
    }

    for laser in ink.lasers {
      guard let first = laser.frames.first, let last = laser.frames.last, t >= first.t, t <= last.t else { continue }
      var lo = 0, hi = laser.frames.count - 1
      while lo < hi {
        let mid = (lo + hi + 1) / 2
        if laser.frames[mid].t <= t { lo = mid } else { hi = mid - 1 }
      }
      let pts = laser.frames[lo].pts
      var points: [(CGPoint, Double)] = []
      var i = 0
      while i + 2 < pts.count {
        points.append((view.point(pts[i], pts[i + 1]), pts[i + 2]))
        i += 3
      }
      drawLaser(ctx, points: points, unit: k * s)
    }

    // Clicks pulse once, small and thin, and are gone in 350 ms; nothing click-related lingers
    // over what's shown (`render::draw_click_ripple` is the keyframe's still version).
    let blue = (r: 0.0, g: 136.0 / 255, b: 1.0)
    func ring(_ c: CGPoint, _ r: Double, _ width: Double, _ color: CGColor) {
      ctx.setLineWidth(width)
      ctx.setStrokeColor(color)
      ctx.strokeEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
    }
    func dot(_ c: CGPoint, _ r: Double, _ color: CGColor) {
      ctx.setFillColor(color)
      ctx.fillEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
    }
    let u = k * s
    for ripple in ink.ripples {
      let age = t - ripple.t
      guard age >= 0, age < 350 else { continue }
      let p = age / 350
      let grow = 1 - pow(1 - p, 3)
      let a = (1 - p) * (1 - p)
      let c = view.point(ripple.x, ripple.y)
      let radius = (4 + 10 * grow) * u
      ring(c, radius + 1.1 * u, 0.9 * u, CGColor(red: 1, green: 1, blue: 1, alpha: 0.35 * a))
      ring(c, radius, 1.3 * u, CGColor(red: blue.r, green: blue.g, blue: blue.b, alpha: 0.75 * a))
    }

    for trail in ink.trails {
      guard t >= trail.t else { continue }
      let alpha = fade(t, at: trail.t1 + 100, over: 250)
      guard alpha > 0.002 else { continue }
      let progress = trail.t1 > trail.t ? min(1, (t - trail.t) / (trail.t1 - trail.t)) : 1
      var points: [CGPoint] = []
      var i = 0
      while i + 1 < trail.pts.count {
        points.append(view.point(trail.pts[i], trail.pts[i + 1]))
        i += 2
      }
      guard points.count >= 2 else { continue }
      let upto = progress * Double(points.count - 1)
      let whole = Int(upto)
      var shown = Array(points[0...whole])
      if whole + 1 < points.count {
        let f = upto - Double(whole)
        let (a, b) = (points[whole], points[whole + 1])
        shown.append(CGPoint(x: a.x + (b.x - a.x) * f, y: a.y + (b.y - a.y) * f))
      }
      for (width, color) in [(3.0, CGColor(red: 1, green: 1, blue: 1, alpha: 0.22 * alpha)),
                             (1.6, CGColor(red: blue.r, green: blue.g, blue: blue.b, alpha: 0.7 * alpha))] {
        ctx.setLineWidth(width * u)
        ctx.setStrokeColor(color)
        ctx.beginPath()
        ctx.addLines(between: shown)
        ctx.strokePath()
      }
      let (start, head) = (shown[0], shown[shown.count - 1])
      dot(start, 1.3 * u, CGColor(red: blue.r, green: blue.g, blue: blue.b, alpha: 0.9 * alpha))
      dot(head, 2.0 * u, CGColor(red: blue.r, green: blue.g, blue: blue.b, alpha: 0.8 * alpha))
    }
  }

  /// A tapered capsule from `a` (radius r0) to `b` (radius r1).
  static func capsule(_ ctx: CGContext, _ a: CGPoint, _ b: CGPoint, _ r0: Double, _ r1: Double, _ color: CGColor) {
    ctx.setFillColor(color)
    let dx = b.x - a.x, dy = b.y - a.y
    let length = hypot(dx, dy)
    if length > 0.001 {
      let nx = -dy / length, ny = dx / length
      ctx.beginPath()
      ctx.move(to: CGPoint(x: a.x + nx * r0, y: a.y + ny * r0))
      ctx.addLine(to: CGPoint(x: b.x + nx * r1, y: b.y + ny * r1))
      ctx.addLine(to: CGPoint(x: b.x - nx * r1, y: b.y - ny * r1))
      ctx.addLine(to: CGPoint(x: a.x - nx * r0, y: a.y - ny * r0))
      ctx.closePath()
      ctx.fillPath()
    }
    ctx.fillEllipse(in: CGRect(x: a.x - r0, y: a.y - r0, width: 2 * r0, height: 2 * r0))
    ctx.fillEllipse(in: CGRect(x: b.x - r1, y: b.y - r1, width: 2 * r1, height: 2 * r1))
  }

  /// The laser trail with the keyframe renderer's four layers (`render::draw_laser`).
  static func drawLaser(_ ctx: CGContext, points: [(CGPoint, Double)], unit: Double) {
    guard !points.isEmpty else { return }
    func rgb(_ r: Double, _ g: Double, _ b: Double, _ a: Double) -> CGColor {
      CGColor(red: r / 255, green: g / 255, blue: b / 255, alpha: a)
    }
    let layers: [(Double, Double, Double, (Double, Double, Double))] = [
      (7.5, 0.08, 1.1, (255, 22, 38)),
      (4.2, 0.22, 1.2, (255, 36, 48)),
      (2.1, 0.78, 1.3, (255, 59, 48)),
      (0.95, 0.92, 1.5, (255, 245, 245)),
    ]
    for (radius, alpha, power, c) in layers {
      let r = radius * unit
      if points.count == 1 {
        let (p, life) = points[0]
        ctx.setFillColor(rgb(c.0, c.1, c.2, alpha * pow(life, power)))
        ctx.fillEllipse(in: CGRect(x: p.x - r, y: p.y - r, width: 2 * r, height: 2 * r))
      }
      for i in 0..<max(0, points.count - 1) {
        let (a, fa) = points[i], (b, fb) = points[i + 1]
        capsule(ctx, a, b, r * pow(fa, 0.7), r * pow(fb, 0.7), rgb(c.0, c.1, c.2, alpha * pow((fa + fb) / 2, power)))
      }
    }
    if let (tip, life) = points.last, life > 0.2 {
      let heads: [(Double, Double, (Double, Double, Double))] = [
        (9.5, 0.14 * life, (255, 20, 36)),
        (5.5, 0.34 * life, (255, 36, 48)),
        (3.0, 0.88 * life, (255, 59, 48)),
        (1.5, life, (255, 255, 255)),
      ]
      for (radius, alpha, c) in heads {
        let r = radius * unit
        ctx.setFillColor(rgb(c.0, c.1, c.2, alpha))
        ctx.fillEllipse(in: CGRect(x: tip.x - r, y: tip.y - r, width: 2 * r, height: 2 * r))
      }
    }
  }
}

final class ExportFrames: @unchecked Sendable {
  let plan: ExportPlan
  let reader: AVAssetReader
  let output: AVAssetReaderTrackOutput
  let writerInput: AVAssetWriterInput
  let adaptor: AVAssetWriterInputPixelBufferAdaptor
  let ci = CIContext(options: [.cacheIntermediates: false])
  let space709 = CGColorSpace(name: CGColorSpace.itur_709)!
  let space = CGColorSpace(name: CGColorSpace.sRGB)!
  let total: Int
  var tick = 0
  var current: CVPixelBuffer?
  var pending: (CVPixelBuffer, Double)?
  var sourceDone = false

  init(plan: ExportPlan, reader: AVAssetReader, output: AVAssetReaderTrackOutput, writerInput: AVAssetWriterInput, adaptor: AVAssetWriterInputPixelBufferAdaptor) {
    self.plan = plan
    self.reader = reader
    self.output = output
    self.writerInput = writerInput
    self.adaptor = adaptor
    total = Int((Double(plan.durationMs) * Double(plan.fps) / 1000).rounded(.up))
    advance()
  }

  /// Reads the next source frame into `pending` with its time in ms.
  private func advance() {
    guard !sourceDone, let sample = output.copyNextSampleBuffer() else {
      sourceDone = true
      pending = nil
      return
    }
    guard let buffer = CMSampleBufferGetImageBuffer(sample) else { return advance() }
    pending = (buffer, CMSampleBufferGetPresentationTimeStamp(sample).seconds * 1000)
  }

  func cameraRect(at ms: Double) -> [Double] {
    guard let keys = plan.camera, !keys.isEmpty else { return plan.crop }
    var lo = 0, hi = keys.count - 1
    if ms <= keys[0][0] { return Array(keys[0][1...4]) }
    if ms >= keys[hi][0] { return Array(keys[hi][1...4]) }
    while lo < hi - 1 {
      let mid = (lo + hi) / 2
      if keys[mid][0] <= ms { lo = mid } else { hi = mid }
    }
    let (a, b) = (keys[lo], keys[hi])
    let f = (ms - a[0]) / max(1e-6, b[0] - a[0])
    return (1...4).map { a[$0] + (b[$0] - a[$0]) * f }
  }

  /// Appends frames while the encoder wants them. Returns when finished or the input is full.
  func pump(done: () -> Void) {
    while writerInput.isReadyForMoreMediaData {
      if tick >= total {
        writerInput.markAsFinished()
        done()
        return
      }
      let ms = Double(tick) * 1000 / Double(plan.fps)
      while let next = pending, next.1 <= ms + 0.5 {
        current = next.0
        advance()
      }
      if current == nil, let next = pending { current = next.0 }
      guard let source = current, let pool = adaptor.pixelBufferPool else {
        writerInput.markAsFinished()
        done()
        return
      }
      var target: CVPixelBuffer?
      CVPixelBufferPoolCreatePixelBuffer(nil, pool, &target)
      guard let out = target else {
        writerInput.markAsFinished()
        done()
        return
      }
      render(source: source, into: out, ms: ms)
      let time = CMTime(value: CMTimeValue(tick), timescale: CMTimeScale(plan.fps))
      if !adaptor.append(out, withPresentationTime: time) {
        writerInput.markAsFinished()
        done()
        return
      }
      tick += 1
      if tick % 120 == 0 { emit(["progress": Double(tick) / Double(total)]) }
    }
  }

  private func render(source: CVPixelBuffer, into out: CVPixelBuffer, ms: Double) {
    let sw = Double(CVPixelBufferGetWidth(source)), sh = Double(CVPixelBufferGetHeight(source))
    let (ow, oh) = (Double(plan.width), Double(plan.height))
    let rect = cameraRect(at: ms)
    let view = ExportView(sourceW: sw, sourceH: sh, rect: rect, outW: ow, outH: oh)
    // Core Image has y up: the rectangle's bottom edge is `sh - y1`.
    let image = CIImage(cvPixelBuffer: source)
      .transformed(by: CGAffineTransform(translationX: -view.originX, y: -(sh - rect[3] * sh)))
      .transformed(by: CGAffineTransform(scaleX: view.scaleX, y: view.scaleY))
      .cropped(to: CGRect(x: 0, y: 0, width: ow, height: oh))
    ci.render(image, to: out, bounds: CGRect(x: 0, y: 0, width: ow, height: oh), colorSpace: space709)
    CVBufferSetAttachment(out, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2, .shouldPropagate)
    CVBufferSetAttachment(out, kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2, .shouldPropagate)
    CVBufferSetAttachment(out, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_709_2, .shouldPropagate)

    CVPixelBufferLockBaseAddress(out, [])
    defer { CVPixelBufferUnlockBaseAddress(out, []) }
    guard let base = CVPixelBufferGetBaseAddress(out),
          let ctx = CGContext(data: base, width: plan.width, height: plan.height, bitsPerComponent: 8,
                              bytesPerRow: CVPixelBufferGetBytesPerRow(out), space: space,
                              bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
    else { return }
    ctx.translateBy(x: 0, y: oh)
    ctx.scaleBy(x: 1, y: -1)
    ctx.setShouldAntialias(true)
    ExportInk.draw(ctx, t: ms, view: view, ink: plan.ink)
  }
}

enum Exporter {
  static func cm(_ ms: Int) -> CMTime { CMTime(value: CMTimeValue(ms), timescale: 1000) }

  static func run(plan url: URL) async throws {
    let plan = try JSONDecoder().decode(ExportPlan.self, from: Data(contentsOf: url))
    guard plan.crop.count == 4 else { throw HelperError("The plan has no crop") }
    let outURL = URL(filePath: plan.out)
    try? FileManager.default.removeItem(at: outURL)

    let asset = AVURLAsset(url: URL(filePath: plan.video))
    guard let videoTrack = try await asset.loadTracks(withMediaType: .video).first else { throw HelperError("The recording has no video") }

    let reader = try AVAssetReader(asset: asset)
    let videoOut = AVAssetReaderTrackOutput(track: videoTrack, outputSettings: [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
    ])
    videoOut.alwaysCopiesSampleData = false
    reader.add(videoOut)

    let writer = try AVAssetWriter(outputURL: outURL, fileType: .mp4)
    writer.shouldOptimizeForNetworkUse = true
    let hevc = plan.codec == "hevc"
    let pixels = Double(plan.width * plan.height) * Double(plan.fps)
    let bitRate = Int(min(18_000_000, max(1_500_000, pixels * (hevc ? 0.045 : 0.065))))
    var compression: [String: Any] = [
      AVVideoAverageBitRateKey: bitRate,
      AVVideoMaxKeyFrameIntervalDurationKey: 2,
      AVVideoExpectedSourceFrameRateKey: plan.fps,
      AVVideoAllowFrameReorderingKey: true,
    ]
    if !hevc { compression[AVVideoProfileLevelKey] = AVVideoProfileLevelH264HighAutoLevel }
    let videoIn = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: hevc ? AVVideoCodecType.hevc : AVVideoCodecType.h264,
      AVVideoWidthKey: plan.width,
      AVVideoHeightKey: plan.height,
      AVVideoColorPropertiesKey: [
        AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
        AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
        AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
      ],
      AVVideoCompressionPropertiesKey: compression,
    ])
    videoIn.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: videoIn, sourcePixelBufferAttributes: [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
      kCVPixelBufferWidthKey as String: plan.width,
      kCVPixelBufferHeightKey as String: plan.height,
      kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any](),
    ])
    writer.add(videoIn)

    // Audio: the plan's file from `audioStartMs`, laid at the start of the output.
    var audioReader: AVAssetReader?
    var audioOut: AVAssetReaderAudioMixOutput?
    var audioIn: AVAssetWriterInput?
    if let audio = plan.audio, plan.audioLengthMs > 0 {
      let source = AVURLAsset(url: URL(filePath: audio))
      if let track = try await source.loadTracks(withMediaType: .audio).first {
        let composition = AVMutableComposition()
        if let slot = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid) {
          try slot.insertTimeRange(CMTimeRange(start: cm(plan.audioStartMs), duration: cm(plan.audioLengthMs)), of: track, at: .zero)
          let reader = try AVAssetReader(asset: composition)
          let output = AVAssetReaderAudioMixOutput(audioTracks: composition.tracks(withMediaType: .audio), audioSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 48000,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
          ])
          reader.add(output)
          let input = AVAssetWriterInput(mediaType: .audio, outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48000,
            AVNumberOfChannelsKey: 1,
            AVEncoderBitRateKey: 96000,
          ])
          input.expectsMediaDataInRealTime = false
          writer.add(input)
          (audioReader, audioOut, audioIn) = (reader, output, input)
        }
      }
    }

    guard reader.startReading() else { throw HelperError(reader.error?.localizedDescription ?? "Could not read the recording") }
    if let audioReader, !audioReader.startReading() { throw HelperError(audioReader.error?.localizedDescription ?? "Could not read the audio") }
    guard writer.startWriting() else { throw HelperError(writer.error?.localizedDescription ?? "Could not write the movie") }
    writer.startSession(atSourceTime: .zero)

    let frames = ExportFrames(plan: plan, reader: reader, output: videoOut, writerInput: videoIn, adaptor: adaptor)
    nonisolated(unsafe) let audioReaderBox = audioReader
    nonisolated(unsafe) let audioOutBox = audioOut
    nonisolated(unsafe) let audioInBox = audioIn

    await withTaskGroup(of: Void.self) { group in
      group.addTask {
        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
          let queue = DispatchQueue(label: "export.video", qos: .userInitiated)
          videoIn.requestMediaDataWhenReady(on: queue) {
            frames.pump { done.resume() }
          }
        }
      }
      if let audioIn = audioInBox, let audioOut = audioOutBox {
        group.addTask {
          await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            let queue = DispatchQueue(label: "export.audio", qos: .userInitiated)
            audioIn.requestMediaDataWhenReady(on: queue) {
              while audioIn.isReadyForMoreMediaData {
                guard let buffer = audioOut.copyNextSampleBuffer() else {
                  audioIn.markAsFinished()
                  done.resume()
                  return
                }
                if !audioIn.append(buffer) {
                  audioIn.markAsFinished()
                  done.resume()
                  return
                }
              }
            }
          }
        }
      }
    }
    _ = audioReaderBox
    if reader.status == .failed { writer.cancelWriting(); throw HelperError(reader.error?.localizedDescription ?? "Reading the recording failed") }
    await writer.finishWriting()
    guard writer.status == .completed else { throw HelperError(writer.error?.localizedDescription ?? "Writing the movie failed") }

    let written = try await AVURLAsset(url: outURL).load(.duration).seconds
    let expected = Double(plan.durationMs) / 1000
    guard abs(written - expected) < 0.25 else {
      try? FileManager.default.removeItem(at: outURL)
      throw HelperError(String(format: "The export is %.2f s long, the recording %.2f s", written, expected))
    }
    let bytes = (try? outURL.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    emit(["done": ["durationMs": Int((written * 1000).rounded()), "bytes": bytes, "frames": frames.tick]])
  }
}

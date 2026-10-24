import AVFoundation
import Foundation
import UIKit

/// Captures made from what's shared into Scrawl: screenshots become a short movie with a flag
/// on each (so each gets a keyframe) and a name (`⟨Screenshot 2 of 3⟩`); a screen recording goes
/// as it is.
enum SharedCapture {
  static func screenshots(_ files: [URL], note: String?, destination: Destination?, store: CaptureStore = .shared) async throws -> Capture {
    let capture = try store.create(kind: .shared, destination: destination, note: note, state: .recording)
    do {
      let movie = try await StillsMovie.write(files, to: store.movie(capture.id))
      var timeline = Timeline(recordedAt: capture.recordedAt, duration: movie.duration, capture: .shared)
      timeline.session = destination?.session
      timeline.note = note
      timeline.markers = files.indices.map { Double($0) * StillsMovie.hold + 0.5 }
      if files.count > 1 {
        timeline.screens = files.indices.map { (Double($0) * StillsMovie.hold, "Screenshot \($0 + 1) of \(files.count)") }
      }
      try timeline.json().write(to: store.timeline(capture.id), options: .atomic)
      return store.update(capture.id) {
        $0.duration = movie.duration
        $0.screenshots = files.count
        $0.state = .waiting
      } ?? capture
    } catch {
      store.delete(capture.id)
      throw error
    }
  }

  /// A screen recording from Photos. It goes as it is if it fits what the Mac takes; otherwise
  /// it's re-encoded to 1080p HEVC first.
  static func movie(_ source: URL, note: String?, destination: Destination?, hosted: Bool, store: CaptureStore = .shared) async throws -> Capture {
    let capture = try store.create(kind: .shared, destination: destination, note: note, state: .recording)
    do {
      let asset = AVURLAsset(url: source)
      let target = store.movie(capture.id)
      let limit = Uploader.maxPayloadBytes(hosted: hosted) - 1_000_000
      try await export(asset, preset: AVAssetExportPresetPassthrough, to: target)
      if size(of: target) > limit {
        try await export(asset, preset: AVAssetExportPresetHEVC1920x1080, to: target)
      }
      guard size(of: target) <= limit else { throw Uploader.SendError.tooLarge }
      let duration = try await asset.load(.duration).seconds
      let hasAudio = try await !asset.loadTracks(withMediaType: .audio).isEmpty
      var timeline = Timeline(recordedAt: capture.recordedAt, duration: duration, capture: .shared)
      timeline.session = destination?.session
      timeline.note = note
      timeline.audio = hasAudio
      try timeline.json().write(to: store.timeline(capture.id), options: .atomic)
      return store.update(capture.id) {
        $0.duration = duration
        $0.state = .waiting
      } ?? capture
    } catch {
      store.delete(capture.id)
      throw error
    }
  }

  /// A card saying it's a test, sent like a screenshot: it checks the pairing from end to end.
  @MainActor
  static func test(destination: Destination?, store: CaptureStore = .shared) async throws -> Capture {
    let file = FileManager.default.temporaryDirectory.appending(path: "scrawl-test-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    guard let png = testCard().pngData() else { throw CocoaError(.fileWriteUnknown) }
    try png.write(to: file)
    return try await screenshots(
      [file],
      note: "A test capture from Scrawl for iPhone, checking that captures reach this session. Nothing needs changing; just say you got it.",
      destination: destination,
      store: store
    )
  }

  @MainActor
  private static func testCard() -> UIImage {
    let size = CGSize(width: 402, height: 874)
    let format = UIGraphicsImageRendererFormat()
    format.scale = 3
    return UIGraphicsImageRenderer(size: size, format: format).image { context in
      UIColor(red: 0.07, green: 0.07, blue: 0.09, alpha: 1).setFill()
      context.fill(CGRect(origin: .zero, size: size))
      let title = NSAttributedString(string: "Scrawl test capture", attributes: [
        .font: UIFont.systemFont(ofSize: 30, weight: .bold), .foregroundColor: UIColor.white,
      ])
      let detail = NSAttributedString(
        string: "Sent from Scrawl for iPhone at \(Date().formatted(date: .omitted, time: .shortened)). If you can read this, captures from this iPhone reach you.",
        attributes: [.font: UIFont.systemFont(ofSize: 19), .foregroundColor: UIColor(white: 0.75, alpha: 1)]
      )
      title.draw(in: CGRect(x: 32, y: 362, width: size.width - 64, height: 40))
      detail.draw(in: CGRect(x: 32, y: 420, width: size.width - 64, height: 140))
      // A red ring around the title, like a mark drawn in Scrawl.
      UIColor(red: 1, green: 0.23, blue: 0.19, alpha: 1).setStroke()
      let ring = UIBezierPath(ovalIn: CGRect(x: 14, y: 340, width: 318, height: 84))
      ring.lineWidth = 5
      ring.stroke()
    }
  }

  private static func export(_ asset: AVAsset, preset: String, to url: URL) async throws {
    try? FileManager.default.removeItem(at: url)
    guard let session = AVAssetExportSession(asset: asset, presetName: preset) else {
      throw CocoaError(.fileWriteUnknown)
    }
    session.shouldOptimizeForNetworkUse = false
    try await session.export(to: url, as: .mp4)
  }

  private static func size(of url: URL) -> Int64 {
    (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.int64Value ?? .max
  }
}

/// A frame from a movie, small, for lists and previews.
enum Thumbnail {
  static func image(of movie: URL, maxPixels: Int) async -> UIImage? {
    let generator = AVAssetImageGenerator(asset: AVURLAsset(url: movie))
    generator.appliesPreferredTrackTransform = true
    generator.maximumSize = CGSize(width: maxPixels, height: maxPixels)
    guard let (image, _) = try? await generator.image(at: CMTime(seconds: 0.4, preferredTimescale: 600)) else { return nil }
    return UIImage(cgImage: image)
  }

  /// A capture's thumbnail, made once and kept beside it (so it outlives the movie).
  static func image(for capture: Capture, store: CaptureStore = .shared) async -> UIImage? {
    let file = store.thumbnail(capture.id)
    if let cached = UIImage(contentsOfFile: file.path) { return cached }
    guard capture.state != .recording, let image = await image(of: store.movie(capture.id), maxPixels: 240) else { return nil }
    if let data = image.jpegData(compressionQuality: 0.8) { try? data.write(to: file, options: .atomic) }
    return image
  }
}

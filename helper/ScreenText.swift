import AVFoundation
import Vision

/// `scrawl-helper read-text MOVIE --at MS:X0,Y0,X1,Y1 [--at …]`: the text Vision recognises in
/// each region (normalized, top-left origin) of the movie's frame at `MS`, as one JSON array in
/// request order. Each line and word box is normalized to the whole frame, top-left origin, so
/// the caller can compare it with ink and pointer positions directly.
enum ScreenText {
  struct Region {
    let ms: Int
    let rect: CGRect
  }

  /// `7300:0.25,0.20,0.45,0.35` → a region at 7.3 s.
  static func parse(_ arguments: [String]) -> [Region]? {
    var regions: [Region] = []
    var index = 0
    while index < arguments.count {
      defer { index += 1 }
      guard arguments[index] == "--at" else { continue }
      guard index + 1 < arguments.count else { return nil }
      index += 1
      let parts = arguments[index].split(separator: ":")
      guard parts.count == 2, let ms = Int(parts[0]) else { return nil }
      let v = parts[1].split(separator: ",").compactMap { Double($0) }
      // An empty region reads nothing rather than failing the others.
      guard v.count == 4 else { return nil }
      regions.append(Region(ms: ms, rect: CGRect(x: v[0], y: v[1], width: max(0, v[2] - v[0]), height: max(0, v[3] - v[1]))))
    }
    return regions
  }

  static func read(movie: URL, regions: [Region]) async throws -> [[String: Any]] {
    let asset = AVURLAsset(url: movie)
    let duration = try await asset.load(.duration).seconds
    let durationMs = Int((duration.isFinite ? duration : 0) * 1000)
    guard durationMs > 0 else { throw HelperError("Movie has no duration") }
    let generator = AVAssetImageGenerator(asset: asset)
    generator.appliesPreferredTrackTransform = true
    generator.requestedTimeToleranceBefore = CMTime(value: 80, timescale: 1000)
    generator.requestedTimeToleranceAfter = CMTime(value: 80, timescale: 1000)
    // Recognition blocks its thread, so it runs on its own queue, a few at a time: blocking calls
    // on Swift's cooperative pool starve it (16 at once hung on a 16-core Mac). Frames decode one
    // moment at a time, each moment's regions queued as soon as its frame is ready.
    let queue = OperationQueue()
    queue.maxConcurrentOperationCount = 4
    queue.qualityOfService = .userInitiated
    let results = Results(count: regions.count)
    let moments = Dictionary(grouping: regions.enumerated(), by: { $0.element.ms })
    for ms in moments.keys.sorted() {
      let at = CMTime(value: CMTimeValue(min(max(0, ms), durationMs - 1)), timescale: 1000)
      let image = try? await generator.image(at: at).image
      for (index, region) in moments[ms] ?? [] {
        queue.addOperation {
          results.set(index, ["t": ms, "lines": image.map { recognize($0, in: region.rect) } ?? []])
        }
      }
    }
    await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
      queue.addBarrierBlock { done.resume() }
    }
    return results.all
  }

  /// Each region's result, written from the recognition queue.
  private final class Results: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [[String: Any]]

    init(count: Int) { items = Array(repeating: ["lines": [[String: Any]]()], count: count) }

    func set(_ index: Int, _ item: [String: Any]) {
      lock.lock()
      items[index] = item
      lock.unlock()
    }

    var all: [[String: Any]] {
      lock.lock()
      defer { lock.unlock() }
      return items
    }
  }

  /// The user's languages Vision can read (each by exact tag, else by language code).
  private static let languages: [String] = {
    let supported = (try? VNRecognizeTextRequest().supportedRecognitionLanguages()) ?? []
    let code = { (tag: String) in tag.split(separator: "-").first.map(String.init) ?? tag }
    return Locale.preferredLanguages.compactMap { tag in
      supported.first { $0 == tag } ?? supported.first { code($0) == code(tag) }
    }
  }()

  private static func recognize(_ image: CGImage, in rect: CGRect) -> [[String: Any]] {
    let (w, h) = (CGFloat(image.width), CGFloat(image.height))
    let crop = CGRect(x: rect.minX * w, y: rect.minY * h, width: rect.width * w, height: rect.height * h)
      .integral
      .intersection(CGRect(x: 0, y: 0, width: w, height: h))
    guard crop.width >= 8, crop.height >= 8, let cropped = image.cropping(to: crop) else { return [] }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    // Screens are full of code and names that language correction would "fix", and detecting
    // the language costs a third of the time: read in the user's languages instead.
    request.usesLanguageCorrection = false
    if !languages.isEmpty { request.recognitionLanguages = languages }
    let handler = VNImageRequestHandler(cgImage: cropped)
    guard (try? handler.perform([request])) != nil else { return [] }
    // Vision's boxes are normalized to the crop with a bottom-left origin.
    func box(_ b: CGRect) -> [Double] {
      let x0 = (crop.minX + b.minX * crop.width) / w
      let x1 = (crop.minX + b.maxX * crop.width) / w
      let y0 = (crop.minY + (1 - b.maxY) * crop.height) / h
      let y1 = (crop.minY + (1 - b.minY) * crop.height) / h
      return [x0, y0, x1, y1].map { (Double($0) * 10_000).rounded() / 10_000 }
    }
    return (request.results ?? []).compactMap { observation -> [String: Any]? in
      guard let candidate = observation.topCandidates(1).first else { return nil }
      let text = candidate.string
      var words: [[String: Any]] = []
      var start = text.startIndex
      while start < text.endIndex {
        guard let wordStart = text[start...].firstIndex(where: { !$0.isWhitespace }) else { break }
        let wordEnd = text[wordStart...].firstIndex(where: \.isWhitespace) ?? text.endIndex
        if let wordBox = try? candidate.boundingBox(for: wordStart..<wordEnd)?.boundingBox {
          words.append(["text": String(text[wordStart..<wordEnd]), "box": box(wordBox)])
        }
        start = wordEnd
      }
      return [
        "text": text,
        "confidence": (Double(candidate.confidence) * 100).rounded() / 100,
        "box": box(observation.boundingBox),
        "words": words,
      ]
    }
  }
}

@preconcurrency import AVFoundation
import CoreGraphics
import Speech

/// The on-device model that transcribes. On macOS 26: SpeechTranscriber where this Mac supports
/// it, else DictationTranscriber (Macs, and virtual machines, without SpeechTranscriber). On
/// macOS 15: SFSpeechRecognizer's on-device Dictation model (`Recognizer`). Each for the language
/// chosen in Settings (`--locale`) when it's supported, else the user's locale, the nearest
/// supported one in the same language, or en-US. `supported` lists the engine's locales.
struct SpeechChoice {
  enum Engine { case speech, dictation, recognizer }
  let engine: Engine
  let locale: Locale
  let supported: [String]

  var name: String {
    switch engine {
    case .speech: "SpeechTranscriber"
    case .dictation: "DictationTranscriber"
    case .recognizer: "SFSpeechRecognizer"
    }
  }

  #if compiler(>=6.2)
  @available(macOS 26, *)
  func module() -> any SpeechModule {
    engine == .dictation
      ? DictationTranscriber(locale: locale, contentHints: [], transcriptionOptions: [.punctuation], reportingOptions: [], attributeOptions: [.audioTimeRange])
      : SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange])
  }
  #endif
}

/// Apple's speech assets are installed per app: `AssetInventory` reserves the locale for this
/// bundle id (`dev.kitlangton.apparent.helper`) and downloads the model if the Mac doesn't have it.
/// A fresh Mac has none, so Apparent installs it in the background at launch
/// (`apparent-helper speech-model --install`) rather than in the middle of a capture. On macOS 15
/// the same command checks for the Speech Recognition permission and the Dictation model instead.
enum SpeechModel {
  /// SpeechAnalyzer (macOS 26, built with its SDK); otherwise `Recognizer`.
  static var analyzer: Bool {
    #if compiler(>=6.2)
    if #available(macOS 26, *) { return true }
    #endif
    return false
  }

  static func choose(preferred: String? = nil) async -> SpeechChoice? {
    #if compiler(>=6.2)
    if #available(macOS 26, *) { return await analyzerChoice(preferred: preferred) }
    #endif
    return Recognizer.choice(preferred: preferred)
  }

  #if compiler(>=6.2)
  @available(macOS 26, *)
  private static func analyzerChoice(preferred: String?) async -> SpeechChoice? {
    if SpeechTranscriber.isAvailable {
      let supported = await SpeechTranscriber.supportedLocales
      if let locale = await locale(preferred: preferred, in: supported, equivalent: { await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current) }) {
        return SpeechChoice(engine: .speech, locale: locale, supported: supported.map { $0.identifier(.bcp47) }.sorted())
      }
    }
    let dictation = await DictationTranscriber.supportedLocales
    if let locale = await locale(preferred: preferred, in: dictation, equivalent: { await DictationTranscriber.supportedLocale(equivalentTo: Locale.current) }) {
      return SpeechChoice(engine: .dictation, locale: locale, supported: dictation.map { $0.identifier(.bcp47) }.sorted())
    }
    return nil
  }
  #endif

  private static func locale(preferred: String?, in supported: [Locale], equivalent: () async -> Locale?) async -> Locale? {
    if let preferred, let match = supported.first(where: { $0.language.maximalIdentifier == Locale(identifier: preferred).language.maximalIdentifier }) {
      return match
    }
    return pick(supported, equivalent: await equivalent())
  }

  /// `equivalent` only counts when it's really in `supported` (macOS 26 names en_US even where
  /// SpeechTranscriber supports nothing).
  private static func pick(_ supported: [Locale], equivalent: Locale?) -> Locale? {
    let ids = supported.map { $0.identifier(.bcp47) }
    if let equivalent, ids.contains(equivalent.identifier(.bcp47)) { return equivalent }
    let language = Locale.current.language.languageCode
    let region = Locale.current.region
    let sameLanguage = supported.filter { $0.language.languageCode == language }
    if let match = sameLanguage.first(where: { $0.region == region }) ?? sameLanguage.first { return match }
    return supported.first { $0.identifier(.bcp47) == "en-US" }
  }

  /// `installed`, `downloading`, `supported` (can be downloaded) or `unsupported`. On macOS 15
  /// there's nothing to download: `installed`, `permission` (Speech Recognition is off) or
  /// `failed` (no Dictation model), with `error` saying what to turn on.
  static func status(_ choice: SpeechChoice) async -> (status: String, error: String?) {
    #if compiler(>=6.2)
    if #available(macOS 26, *) {
      switch await AssetInventory.status(forModules: [choice.module()]) {
      case .installed: return ("installed", nil)
      case .downloading: return ("downloading", nil)
      case .supported: return ("supported", nil)
      case .unsupported: return ("unsupported", nil)
      @unknown default: return ("unsupported", nil)
      }
    }
    #endif
    if let off = Recognizer.permissionProblem() { return ("permission", off) }
    if let missing = await Recognizer.dictationProblem(choice) { return ("failed", missing) }
    return ("installed", nil)
  }

  /// Every locale the engine knows, for Settings' language menu.
  static func supportedLocales() async -> [String] {
    #if compiler(>=6.2)
    if #available(macOS 26, *) {
      let supported = SpeechTranscriber.isAvailable
        ? await SpeechTranscriber.supportedLocales : await DictationTranscriber.supportedLocales
      return supported.map { $0.identifier(.bcp47) }.sorted()
    }
    #endif
    return SFSpeechRecognizer.supportedLocales().map { $0.identifier(.bcp47) }.sorted()
  }

  static func describe(_ choice: SpeechChoice?, status: String, error: String? = nil, locales: [String] = []) -> [String: Any] {
    guard let choice else { return ["status": "unsupported", "locales": locales] }
    var line: [String: Any] = ["engine": choice.name, "locale": choice.locale.identifier, "status": status, "locales": choice.supported]
    if let error { line["error"] = error }
    return line
  }

  /// `speech-model [--install]`: one status line; with `--install`, installs the model if it
  /// can be downloaded, with progress lines about once a second and a last line that's
  /// `installed` or `failed` (with `error`). Never asks for a permission.
  static func run(install: Bool, preferred: String? = nil) async {
    let choice = await choose(preferred: preferred)
    guard let choice else {
      emit(describe(nil, status: "unsupported", locales: await supportedLocales()))
      return
    }
    let (status, error) = await status(choice)
    emit(describe(choice, status: status, error: error))
    guard install, status == "supported" || status == "downloading" else { return }
    do {
      try await self.install(choice) { fraction in
        var line = describe(choice, status: "downloading")
        line["progress"] = fraction
        emit(line)
      }
      emit(describe(choice, status: "installed"))
    } catch {
      var line = describe(choice, status: "failed")
      line["error"] = error.localizedDescription
      emit(line)
    }
  }

  static func install(_ choice: SpeechChoice, progress: @escaping @Sendable (Double) -> Void) async throws {
    #if compiler(>=6.2)
    if #available(macOS 26, *) {
      // Each language reserves a slot; Apparent uses one language at a time.
      let reserved = await AssetInventory.reservedLocales
      if !reserved.contains(where: { $0.language.maximalIdentifier == choice.locale.language.maximalIdentifier }) {
        var count = reserved.count
        for locale in reserved where count >= AssetInventory.maximumReservedLocales {
          if await AssetInventory.release(reservedLocale: locale) { count -= 1 }
        }
      }
      guard let request = try await AssetInventory.assetInstallationRequest(supporting: [choice.module()]) else { return }
      let reporter = Task {
        while !Task.isCancelled {
          progress(request.progress.fractionCompleted)
          try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
      }
      defer { reporter.cancel() }
      try await request.downloadAndInstall()
    }
    #endif
  }
}

/// macOS 15: SFSpeechRecognizer, on device only so narration never leaves the Mac. There's no
/// AssetInventory before macOS 26, so nothing to download: SFSpeechRecognizer needs the Speech
/// Recognition permission (asked for in onboarding, `Permissions.request`) and Dictation's
/// on-device model, which `speech-model` checks for at launch so neither waits for the first
/// capture. `supportsOnDeviceRecognition` can be true before that model is downloaded, so the
/// check recognizes half a second of silence.
enum Recognizer {
  /// The language chosen in Settings when there's a recognizer for it, else the user's locale
  /// or en-US, preferring one that recognizes on device.
  static func choice(preferred: String? = nil) -> SpeechChoice? {
    let supported = SFSpeechRecognizer.supportedLocales()
    let ids = supported.map { $0.identifier(.bcp47) }.sorted()
    if let preferred,
       let match = supported.first(where: { $0.language.maximalIdentifier == Locale(identifier: preferred).language.maximalIdentifier }),
       let recognizer = SFSpeechRecognizer(locale: match)
    {
      return SpeechChoice(engine: .recognizer, locale: recognizer.locale, supported: ids)
    }
    let recognizers = [Locale.current, Locale(identifier: "en-US")].compactMap { SFSpeechRecognizer(locale: $0) }
    guard let recognizer = recognizers.first(where: \.supportsOnDeviceRecognition) ?? recognizers.first else { return nil }
    return SpeechChoice(engine: .recognizer, locale: recognizer.locale, supported: ids)
  }

  /// What to turn on when Speech Recognition isn't allowed, or nil when it is. Never prompts.
  static func permissionProblem() -> String? {
    switch SFSpeechRecognizer.authorizationStatus() {
    case .authorized: return nil
    case .notDetermined: return "Speech Recognition isn't allowed yet, so captures have no transcript."
    default: return "Speech Recognition is off. Turn it on in System Settings › Privacy & Security."
    }
  }

  /// Dictation's on-device model is missing (or broken), or nil when it recognizes.
  static func dictationProblem(_ choice: SpeechChoice) async -> String? {
    let dictationOff = "Turn on Dictation in System Settings › Keyboard for transcripts."
    guard let recognizer = SFSpeechRecognizer(locale: choice.locale), recognizer.supportsOnDeviceRecognition else { return dictationOff }
    let format = AVAudioFormat(standardFormatWithSampleRate: 16000, channels: 1)!
    guard let silence = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 8000) else { return dictationOff }
    silence.frameLength = 8000
    let request = SFSpeechAudioBufferRecognitionRequest()
    request.append(silence)
    request.endAudio()
    do {
      _ = try await recognize(request, with: recognizer, timeout: 10)
      return nil
    } catch {
      return dictationOff
    }
  }

  /// Chunks run one after another; the recognizer is not built for concurrent tasks. Without
  /// permission (`missing: permission`) or the Dictation model (`dictation`) the output says so
  /// rather than asking now.
  static func transcribe(_ url: URL, choice: SpeechChoice) async throws -> [String: Any] {
    let (status, error) = await SpeechModel.status(choice)
    if status != "installed" {
      var result = Transcriber.missing(choice, status == "failed" ? "dictation" : status)
      result["error"] = error
      return result
    }
    guard let recognizer = SFSpeechRecognizer(locale: choice.locale) else { return Transcriber.missing(choice, "unsupported") }
    let chunks = try? Transcriber.splitWavAtSilences(url)
    defer { if let chunks { try? FileManager.default.removeItem(at: chunks.dir) } }
    let files = chunks?.files ?? [(offsetMs: 0, url: url)]
    var segments: [[String: Any]] = []
    var words: [[String: Any]] = []
    for item in files {
      let request = SFSpeechURLRecognitionRequest(url: item.url)
      request.addsPunctuation = true
      request.taskHint = .dictation
      let (segs, wrds) = split(try await recognize(request, with: recognizer, timeout: 120), offsetMs: item.offsetMs)
      segments.append(contentsOf: segs)
      words.append(contentsOf: wrds)
    }
    return Transcriber.result(choice, segments: segments, words: words)
  }

  /// The final transcription, or nil for audio without speech. Cancelled after `timeout` seconds.
  private static func recognize(_ request: SFSpeechRecognitionRequest, with recognizer: SFSpeechRecognizer, timeout: Double) async throws -> SFTranscription? {
    request.requiresOnDeviceRecognition = true
    request.shouldReportPartialResults = false
    recognizer.queue = OperationQueue()
    return try await withCheckedThrowingContinuation { cont in
      let lock = NSLock()
      var resumed = false
      func finish(_ outcome: Result<SFTranscription?, Error>) {
        lock.lock(); defer { lock.unlock() }
        guard !resumed else { return }
        resumed = true
        cont.resume(with: outcome)
      }
      let task = recognizer.recognitionTask(with: request) { result, error in
        if let result, result.isFinal {
          finish(.success(result.bestTranscription))
        } else if let error {
          // 1110: "No speech detected", which is a silent chunk rather than a failure.
          let ns = error as NSError
          finish(ns.domain == "kAFAssistantErrorDomain" && ns.code == 1110 ? .success(nil) : .failure(error))
        }
      }
      DispatchQueue.global().asyncAfter(deadline: .now() + timeout) {
        task.cancel()
        finish(.failure(HelperError("Speech recognition took longer than \(Int(timeout)) s")))
      }
    }
  }

  /// SFSpeechRecognizer's segments are words; a pause of 0.8 s or more starts a new segment,
  /// roughly where SpeechAnalyzer would.
  private static func split(_ transcription: SFTranscription?, offsetMs: Int) -> ([[String: Any]], [[String: Any]]) {
    guard let transcription else { return ([], []) }
    var words: [[String: Any]] = []
    var segments: [[String: Any]] = []
    var current: [(t0: Int, t1: Int, text: String)] = []
    func closeSegment() {
      guard let first = current.first, let last = current.last else { return }
      segments.append(["t0": first.t0, "t1": last.t1, "text": current.map(\.text).joined(separator: " ")])
      current.removeAll()
    }
    for segment in transcription.segments {
      let text = segment.substring.trimmingCharacters(in: .whitespacesAndNewlines)
      guard !text.isEmpty else { continue }
      let t0 = Int((segment.timestamp * 1000).rounded()) + offsetMs
      let t1 = Int(((segment.timestamp + segment.duration) * 1000).rounded()) + offsetMs
      if let last = current.last, t0 - last.t1 >= 800 { closeSegment() }
      words.append(["t0": t0, "t1": t1, "text": text])
      current.append((t0, t1, text))
    }
    closeSegment()
    return (segments, words)
  }
}

/// On-device transcription with SpeechAnalyzer on macOS 26 (finalized results only, each word with
/// its own audio time range), or `Recognizer` on macOS 15. Without a usable model the output says
/// why in `missing` (`downloading`, `unsupported` or `failed`, with `error`) and has no text,
/// rather than failing.
enum Transcriber {
  static func transcribe(_ url: URL, preferred: String? = nil) async throws -> [String: Any] {
    guard let choice = await SpeechModel.choose(preferred: preferred) else {
      return missing(nil, "unsupported")
    }
    #if compiler(>=6.2)
    if #available(macOS 26, *) { return try await analyzerTranscribe(url, choice: choice) }
    #endif
    return try await Recognizer.transcribe(url, choice: choice)
  }

  #if compiler(>=6.2)
  @available(macOS 26, *)
  private static func analyzerTranscribe(_ url: URL, choice: SpeechChoice) async throws -> [String: Any] {
    switch await SpeechModel.status(choice).status {
    case "installed": break
    case "downloading": return missing(choice, "downloading")
    case "unsupported": return missing(choice, "unsupported")
    default:
      // Supported but not installed for Apparent: usually on disk already and only needs
      // reserving, which is quick. A real download is left to the background install.
      do {
        let done = try await withThrowingTaskGroup(of: Bool.self) { group in
          group.addTask { try await SpeechModel.install(choice) { _ in }; return true }
          group.addTask { try await Task.sleep(nanoseconds: 15_000_000_000); return false }
          let first = try await group.next() ?? false
          group.cancelAll()
          return first
        }
        if !done { return missing(choice, "downloading") }
      } catch {
        var result = missing(choice, "failed")
        result["error"] = error.localizedDescription
        return result
      }
    }
    let (segments, words): ([[String: Any]], [[String: Any]])
    if let chunks = try? splitWavAtSilences(url), chunks.files.count > 1 {
      defer { try? FileManager.default.removeItem(at: chunks.dir) }
      let ordered = try await withThrowingTaskGroup(of: (Int, [[String: Any]], [[String: Any]]).self) { group in
        for (idx, item) in chunks.files.enumerated() {
          group.addTask {
            let (segs, wrds) = try await transcribeSingle(item.url, choice: choice, offsetMs: item.offsetMs)
            return (idx, segs, wrds)
          }
        }
        var collected: [(Int, [[String: Any]], [[String: Any]])] = []
        for try await res in group {
          collected.append(res)
        }
        return collected.sorted { $0.0 < $1.0 }
      }
      segments = ordered.flatMap(\.1)
      words = ordered.flatMap(\.2)
    } else {
      (segments, words) = try await transcribeSingle(url, choice: choice, offsetMs: 0)
    }
    return result(choice, segments: segments, words: words)
  }

  @available(macOS 26, *)
  private static func transcribeSingle(_ url: URL, choice: SpeechChoice, offsetMs: Int) async throws -> ([[String: Any]], [[String: Any]]) {
    let file = try AVAudioFile(forReading: url)
    var segments: [[String: Any]] = []
    var words: [[String: Any]] = []
    func collect(_ text: AttributedString, _ range: CMTimeRange) {
      let line = String(text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
      guard !line.isEmpty else { return }
      for run in text.runs {
        guard let time = run.audioTimeRange else { continue }
        let word = String(text[run.range].characters).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !word.isEmpty else { continue }
        words.append(["t0": ms(time.start) + offsetMs, "t1": ms(time.end) + offsetMs, "text": word])
      }
      segments.append(["t0": ms(range.start) + offsetMs, "t1": ms(range.end) + offsetMs, "text": line])
    }
    let module = choice.module()
    let analyzer = try await SpeechAnalyzer(inputAudioFile: file, modules: [module], finishAfterFile: true)
    if let transcriber = module as? SpeechTranscriber {
      for try await result in transcriber.results { collect(result.text, result.range) }
    } else if let transcriber = module as? DictationTranscriber {
      for try await result in transcriber.results where result.isFinal { collect(result.text, result.range) }
    }
    withExtendedLifetime(analyzer) {}
    return (segments, words)
  }
  #endif

  static func result(_ choice: SpeechChoice, segments: [[String: Any]], words: [[String: Any]]) -> [String: Any] {
    [
      "engine": choice.name,
      "locale": choice.locale.identifier,
      "text": segments.compactMap { $0["text"] as? String }.joined(separator: " "),
      "segments": segments,
      "words": words,
    ]
  }

  static func missing(_ choice: SpeechChoice?, _ reason: String) -> [String: Any] {
    var result: [String: Any] = ["locale": choice?.locale.identifier ?? Locale.current.identifier, "text": "", "segments": [], "words": [], "missing": reason]
    if let choice { result["engine"] = choice.name }
    return result
  }

  struct WavChunks {
    let dir: URL
    let files: [(offsetMs: Int, url: URL)]
  }

  /// Splits a 16 kHz mono 16-bit WAV longer than 22 s at silence midpoints (>= 300 ms) so
  /// long recordings transcribe across concurrent SpeechAnalyzer instances without cutting words.
  static func splitWavAtSilences(_ url: URL) throws -> WavChunks? {
    let attrs = try FileManager.default.attributesOfItem(atPath: url.path)
    guard let fileSize = attrs[.size] as? Int, fileSize >= 22 * 32000 + 44 else { return nil }
    let data = try Data(contentsOf: url)
    guard data.count >= 44 else { return nil }
    let pcmStart = 44
    let pcmBytes = data.count - pcmStart
    let sampleCount = pcmBytes / 2
    let durationMs = sampleCount / 16
    guard durationMs >= 22000 else { return nil }

    let windowSamples = 800 // 50 ms at 16 kHz
    let thresholdSq = (0.012 * 32768.0) * (0.012 * 32768.0)
    var silences: [(midMs: Int, lenMs: Int)] = []
    data.withUnsafeBytes { rawBuf in
      let base = rawBuf.baseAddress!.advanced(by: pcmStart).assumingMemoryBound(to: Int16.self)
      var silenceStartMs: Int? = nil
      var i = 0
      while i + windowSamples <= sampleCount {
        var sumSq: Double = 0
        for j in 0..<windowSamples {
          let v = Double(Int16(littleEndian: base[i + j]))
          sumSq += v * v
        }
        let meanSq = sumSq / Double(windowSamples)
        let tMs = i / 16
        if meanSq <= thresholdSq {
          if silenceStartMs == nil { silenceStartMs = tMs }
        } else if let s0 = silenceStartMs {
          if tMs - s0 >= 300 {
            silences.append(((s0 + tMs) / 2, tMs - s0))
          }
          silenceStartMs = nil
        }
        i += windowSamples
      }
    }
    guard !silences.isEmpty else { return nil }
    let targetChunks = min(6, max(2, (durationMs + 19999) / 20000))
    var cuts = [0]
    for k in 1..<targetChunks {
      let ideal = (durationMs * k) / targetChunks
      let candidates = silences.filter { abs($0.midMs - ideal) <= 8000 && $0.midMs > cuts.last! + 5000 && $0.midMs < durationMs - 5000 }
      if let best = candidates.min(by: { abs($0.midMs - ideal) < abs($1.midMs - ideal) }) {
        cuts.append(best.midMs)
      }
    }
    cuts.append(durationMs)
    guard cuts.count > 2 else { return nil }

    let dir = FileManager.default.temporaryDirectory.appending(path: "apparent-wav-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    var files: [(offsetMs: Int, url: URL)] = []
    for idx in 0..<(cuts.count - 1) {
      let c0 = cuts[idx]
      let c1 = cuts[idx + 1]
      let byte0 = pcmStart + c0 * 32
      let byte1 = min(data.count, pcmStart + c1 * 32)
      guard byte1 > byte0 else { continue }
      let chunkUrl = dir.appending(path: "chunk-\(idx).wav")
      try Media.wav16kMono(data[byte0..<byte1]).write(to: chunkUrl)
      files.append((c0, chunkUrl))
    }
    return WavChunks(dir: dir, files: files)
  }

  private static func ms(_ time: CMTime) -> Int {
    time.seconds.isFinite ? max(0, Int((time.seconds * 1000).rounded())) : 0
  }
}

enum Media {
  static func probe(_ movie: URL) async throws -> [String: Any] {
    let asset = AVURLAsset(url: movie)
    let duration = try await asset.load(.duration).seconds
    var result: [String: Any] = ["durationMs": Int(((duration.isFinite ? duration : 0) * 1000).rounded())]
    var videoDuration = duration
    if let track = try await asset.loadTracks(withMediaType: .video).first {
      let (size, transform) = try await track.load(.naturalSize, .preferredTransform)
      let shown = size.applying(transform)
      result["width"] = Int(abs(shown.width).rounded())
      result["height"] = Int(abs(shown.height).rounded())
      result["fps"] = Double(try await track.load(.nominalFrameRate))
      let range = try await track.load(.timeRange)
      videoDuration = range.duration.seconds
      if videoDuration.isFinite && videoDuration > 0 {
        result["videoDurationMs"] = Int((videoDuration * 1000).rounded())
      }
    }
    let audioTracks = try await asset.loadTracks(withMediaType: .audio)
    result["audioTracks"] = audioTracks.count
    var audioOverlapsVideo = false
    for track in audioTracks {
      if try await audioTrackOverlaps(track, start: 0, end: videoDuration) {
        audioOverlapsVideo = true
      }
    }
    result["audioOverlapsVideo"] = audioOverlapsVideo
    return result
  }

  /// A track's timeRange can include empty edit-list padding before its media.
  /// Admission and export must both find an actual, nonempty media segment.
  static func audioTrackOverlaps(_ track: AVAssetTrack, start: Double, end: Double) async throws -> Bool {
    for segment in try await track.load(.segments) where !segment.isEmpty {
      let range = segment.timeMapping.target
      let first = range.start.seconds
      let last = CMTimeRangeGetEnd(range).seconds
      if first.isFinite && last.isFinite && first < end && last > start && last > first {
        return true
      }
    }
    return false
  }

  /// Decodes frames as raw RGBA8 (`frame-NNNN.rgba`), either sampled every `everyMs` (nearest
  /// decodable frame, for keyframe candidates) or exactly at `atMs` (for the keyframes themselves).
  static func frames(movie: URL, outDir: URL, everyMs: Int?, atMs: [Int]?, maxEdge: Int) async throws -> [[String: Any]] {
    let asset = AVURLAsset(url: movie)
    let duration = try await asset.load(.duration).seconds
    let durationMs = Int((duration.isFinite ? duration : 0) * 1000)
    guard durationMs > 0 else { throw HelperError("Movie has no duration") }
    try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

    let isEvery = (everyMs ?? 0) > 0
    let requested: [Int] = if let everyMs, everyMs > 0 {
      Array(stride(from: 0, to: durationMs, by: everyMs))
    } else {
      (atMs ?? []).map { min(max(0, $0), max(0, durationMs - 1)) }
    }
    guard !requested.isEmpty else { return [] }
    let tolerance = isEvery
      ? CMTime(value: CMTimeValue((everyMs ?? 1) / 2), timescale: 1000)
      : CMTime(value: 80, timescale: 1000)
    let workerCount = min(6, requested.count)
    let chunkSize = (requested.count + workerCount - 1) / workerCount

    let indexed = try await withThrowingTaskGroup(of: [(Int, [String: Any])].self) { group in
      for worker in 0..<workerCount {
        let start = worker * chunkSize
        let end = min(start + chunkSize, requested.count)
        if start >= end { continue }
        let sliceReq = Array(requested[start..<end])
        group.addTask {
          let generator = AVAssetImageGenerator(asset: asset)
          generator.appliesPreferredTrackTransform = true
          if maxEdge > 0 { generator.maximumSize = CGSize(width: maxEdge, height: maxEdge) }
          generator.requestedTimeToleranceBefore = tolerance
          generator.requestedTimeToleranceAfter = tolerance
          let sliceTimes = sliceReq.map { CMTime(value: CMTimeValue($0), timescale: 1000) }
          var local: [(Int, [String: Any])] = []
          var offset = 0
          for await result in generator.images(for: sliceTimes) {
            let curIndex = start + offset
            offset += 1
            guard let image = try? result.image else { continue }
            let reqTime = result.requestedTime.seconds.isFinite
              ? Int((result.requestedTime.seconds * 1000).rounded())
              : sliceReq[min(offset - 1, sliceReq.count - 1)]
            let actual = (try? result.actualTime).map { Int(($0.seconds * 1000).rounded()) } ?? reqTime
            let file = outDir.appending(path: String(format: "frame-%04d.rgba", curIndex))
            try writeRGBA(image, to: file)
            local.append((curIndex, ["t": reqTime, "actual": actual, "width": image.width, "height": image.height, "file": file.path]))
          }
          return local
        }
      }
      var pairs: [(Int, [String: Any])] = []
      for try await batch in group {
        pairs.append(contentsOf: batch)
      }
      return pairs
    }
    return indexed.sorted { $0.0 < $1.0 }.map(\.1)
  }

  private static let srgbSpace = CGColorSpace(name: CGColorSpace.sRGB)!

  private static func writeRGBA(_ image: CGImage, to url: URL) throws {
    let width = image.width, height = image.height
    var pixels = Data(count: width * height * 4)
    let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
      guard let context = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                    space: srgbSpace, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
      else { return false }
      context.interpolationQuality = .high
      context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
      return true
    }
    guard drawn else { throw HelperError("Could not convert frame") }
    try pixels.write(to: url)
  }

  /// Re-encodes a movie's video to HEVC with its long edge at most `maxEdge`, copying every audio
  /// track as it is. Each sample keeps its presentation time, so the timeline (and every time in
  /// `apparent.json`) still lines up. Fails rather than writing a movie of a different length.
  static func shrink(movie: URL, out: URL, maxEdge: Int, bitRate: Int) async throws -> [String: Any] {
    let started = Date()
    let asset = AVURLAsset(url: movie)
    guard let videoTrack = try await asset.loadTracks(withMediaType: .video).first else { throw HelperError("Movie has no video") }
    let audioTracks = try await asset.loadTracks(withMediaType: .audio)
    let (natural, transform) = try await videoTrack.load(.naturalSize, .preferredTransform)
    let duration = try await asset.load(.duration)
    let (w, h) = (abs(natural.width), abs(natural.height))
    let scale = min(1, Double(maxEdge) / Double(max(w, h)))
    func even(_ v: Double) -> Int { max(2, Int((v / 2).rounded()) * 2) }
    let (outW, outH) = (even(w * scale), even(h * scale))

    try? FileManager.default.removeItem(at: out)
    let reader = try AVAssetReader(asset: asset)
    let writer = try AVAssetWriter(outputURL: out, fileType: .mov)
    writer.movieTimeScale = duration.timescale
    writer.metadata = (try? await asset.load(.metadata)) ?? []

    let videoOut = AVAssetReaderTrackOutput(track: videoTrack, outputSettings: [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
      kCVPixelBufferWidthKey as String: outW,
      kCVPixelBufferHeightKey as String: outH,
    ])
    videoOut.alwaysCopiesSampleData = false
    reader.add(videoOut)
    let videoIn = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: AVVideoCodecType.hevc,
      AVVideoWidthKey: outW,
      AVVideoHeightKey: outH,
      AVVideoCompressionPropertiesKey: [
        AVVideoAverageBitRateKey: bitRate,
        AVVideoMaxKeyFrameIntervalDurationKey: 2,
        AVVideoAllowFrameReorderingKey: true,
      ],
    ])
    videoIn.transform = transform
    videoIn.expectsMediaDataInRealTime = false
    videoIn.mediaTimeScale = try await videoTrack.load(.naturalTimeScale)
    writer.add(videoIn)

    var pairs: [(AVAssetReaderOutput, AVAssetWriterInput)] = [(videoOut, videoIn)]
    for track in audioTracks {
      let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
      reader.add(output)
      let hint = try await track.load(.formatDescriptions).first
      let input = AVAssetWriterInput(mediaType: .audio, outputSettings: nil, sourceFormatHint: hint)
      input.expectsMediaDataInRealTime = false
      writer.add(input)
      pairs.append((output, input))
    }

    guard reader.startReading() else { throw HelperError(reader.error?.localizedDescription ?? "Could not read the movie") }
    guard writer.startWriting() else { throw HelperError(writer.error?.localizedDescription ?? "Could not write the movie") }
    writer.startSession(atSourceTime: .zero)

    await withTaskGroup(of: Void.self) { group in
      for (index, (output, input)) in pairs.enumerated() {
        nonisolated(unsafe) let output = output
        nonisolated(unsafe) let input = input
        nonisolated(unsafe) let reader = reader
        group.addTask {
          await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            let queue = DispatchQueue(label: "shrink.\(index)", qos: .utility)
            input.requestMediaDataWhenReady(on: queue) {
              while input.isReadyForMoreMediaData {
                guard let buffer = output.copyNextSampleBuffer() else {
                  input.markAsFinished()
                  done.resume()
                  return
                }
                if !input.append(buffer) {
                  reader.cancelReading()
                  input.markAsFinished()
                  done.resume()
                  return
                }
              }
            }
          }
        }
      }
    }
    if reader.status == .failed { writer.cancelWriting(); throw HelperError(reader.error?.localizedDescription ?? "Reading the movie failed") }
    await writer.finishWriting()
    guard writer.status == .completed else { throw HelperError(writer.error?.localizedDescription ?? "Writing the movie failed") }

    let written = try await AVURLAsset(url: out).load(.duration)
    let drift = abs(written.seconds - duration.seconds)
    guard drift < 0.1 else {
      try? FileManager.default.removeItem(at: out)
      throw HelperError(String(format: "The shrunk movie is %.2f s off the original", drift))
    }
    func size(_ url: URL) -> Int { (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0 }
    return [
      "before": size(movie),
      "after": size(out),
      "width": outW,
      "height": outH,
      "durationMs": Int((written.seconds * 1000).rounded()),
      "ms": Int(Date().timeIntervalSince(started) * 1000),
    ]
  }

  /// Every audio track mixed to 16 kHz mono 16-bit PCM, padded so sample 0 is the movie's start.
  static func exportAudio(movie: URL, wav: URL) async throws -> [String: Any] {
    let asset = AVURLAsset(url: movie)
    let tracks = try await asset.loadTracks(withMediaType: .audio)
    guard !tracks.isEmpty else { throw HelperError("Movie has no audio") }
    let reader = try AVAssetReader(asset: asset)
    let output = AVAssetReaderAudioMixOutput(audioTracks: tracks, audioSettings: [
      AVFormatIDKey: kAudioFormatLinearPCM,
      AVSampleRateKey: 16000,
      AVNumberOfChannelsKey: 1,
      AVLinearPCMBitDepthKey: 16,
      AVLinearPCMIsFloatKey: false,
      AVLinearPCMIsBigEndianKey: false,
      AVLinearPCMIsNonInterleaved: false,
    ])
    reader.add(output)
    guard reader.startReading() else { throw HelperError(reader.error?.localizedDescription ?? "Could not read audio") }
    var pcm = Data()
    var first = true
    while let buffer = output.copyNextSampleBuffer() {
      if first {
        first = false
        let start = CMSampleBufferGetPresentationTimeStamp(buffer).seconds
        if start.isFinite, start > 0 { pcm.append(Data(count: Int(start * 16000) * 2)) }
      }
      guard let block = CMSampleBufferGetDataBuffer(buffer) else { continue }
      let length = CMBlockBufferGetDataLength(block)
      var chunk = Data(count: length)
      chunk.withUnsafeMutableBytes { _ = CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: $0.baseAddress!) }
      pcm.append(chunk)
    }
    if reader.status == .failed { throw HelperError(reader.error?.localizedDescription ?? "Audio export failed") }
    try wav16kMono(pcm).write(to: wav)
    return ["path": wav.path, "samples": pcm.count / 2, "durationMs": pcm.count / 32]
  }

  static func wav16kMono(_ pcm: Data) -> Data {
    var out = Data(capacity: 44 + pcm.count)
    func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { out.append(contentsOf: $0) } }
    func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { out.append(contentsOf: $0) } }
    out.append(contentsOf: Array("RIFF".utf8)); u32(UInt32(36 + pcm.count))
    out.append(contentsOf: Array("WAVE".utf8))
    out.append(contentsOf: Array("fmt ".utf8)); u32(16); u16(1); u16(1); u32(16000); u32(32000); u16(2); u16(16)
    out.append(contentsOf: Array("data".utf8)); u32(UInt32(pcm.count))
    out.append(pcm)
    return out
  }
}

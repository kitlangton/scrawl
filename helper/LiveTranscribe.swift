@preconcurrency import AVFoundation
import Speech

/// `transcribe-stream`: transcribes a recording while it's made. Reads 16 kHz mono 16-bit
/// little-endian PCM on stdin from the recording's first sample (the samples `audio.wav` will
/// hold), and writes JSON lines: `{"ready":true,"locale"}` once the model is loaded, one
/// `{"segment":{t0,t1,text},"words":[…]}` per finalized result, and after stdin closes and the
/// tail is finalized `{"done":true,"samples":N}`. Times are ms from the first sample, as
/// `transcribe` gives them for the WAV. It never downloads a model: one that isn't installed
/// fails at once, and Scrawl transcribes the finished recording instead.
/// Before macOS 26 there is no SpeechAnalyzer to stream to, so it fails at once the same way.
enum LiveTranscriber {
  static func run(preferred: String? = nil) async throws {
    #if compiler(>=6.2)
    if #available(macOS 26, *) { return try await stream(preferred: preferred) }
    #endif
    throw HelperError("live transcription needs macOS 26's SpeechAnalyzer")
  }

  #if compiler(>=6.2)
  @available(macOS 26, *)
  private static func stream(preferred: String?) async throws {
    guard let choice = await SpeechModel.choose(preferred: preferred), choice.engine == .speech else {
      throw HelperError("SpeechTranscriber isn't available on this Mac")
    }
    let locale = choice.locale
    let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange])
    guard await AssetInventory.status(forModules: [transcriber]) == .installed else {
      throw HelperError("the speech model for \(locale.identifier) isn't installed")
    }

    let pcm = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1, interleaved: false)!
    let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) ?? pcm
    let converter = format == pcm ? nil : AVAudioConverter(from: pcm, to: format)
    if format != pcm && converter == nil { throw HelperError("can't convert audio to \(format)") }

    let analyzer = SpeechAnalyzer(modules: [transcriber], options: SpeechAnalyzer.Options(priority: .userInitiated, modelRetention: .whileInUse))
    let (inputs, continuation) = AsyncStream<AnalyzerInput>.makeStream()
    let results = Task {
      for try await result in transcriber.results {
        let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { continue }
        var words: [[String: Any]] = []
        for run in result.text.runs {
          guard let range = run.audioTimeRange else { continue }
          let word = String(result.text[run.range].characters).trimmingCharacters(in: .whitespacesAndNewlines)
          guard !word.isEmpty else { continue }
          words.append(["t0": ms(range.start), "t1": ms(range.end), "text": word])
        }
        emit(["segment": ["t0": ms(result.range.start), "t1": ms(result.range.end), "text": text], "words": words])
      }
    }

    // Audio arrives before the model is ready; the stream holds it until the analyzer starts.
    let reader = StdinPCM(format: pcm, converter: converter, output: format, continuation: continuation)
    let samples = Task.detached { try reader.pump() }

    try await analyzer.prepareToAnalyze(in: format)
    emit(["ready": true, "locale": locale.identifier])
    try await analyzer.start(inputSequence: inputs)
    let total = try await samples.value
    try await analyzer.finalizeAndFinishThroughEndOfInput()
    try await results.value
    emit(["done": true, "samples": total])
  }

  /// As `Transcriber` rounds them, so both paths give the same times.
  private static func ms(_ time: CMTime) -> Int {
    time.seconds.isFinite ? max(0, Int((time.seconds * 1000).rounded())) : 0
  }
  #endif
}

#if compiler(>=6.2)
/// Turns stdin into analyzer buffers, about 100 ms at a time.
@available(macOS 26, *)
private final class StdinPCM: @unchecked Sendable {
  let format: AVAudioFormat
  let converter: AVAudioConverter?
  let output: AVAudioFormat
  let continuation: AsyncStream<AnalyzerInput>.Continuation

  init(format: AVAudioFormat, converter: AVAudioConverter?, output: AVAudioFormat, continuation: AsyncStream<AnalyzerInput>.Continuation) {
    self.format = format
    self.converter = converter
    self.output = output
    self.continuation = continuation
  }

  /// Reads to the end of stdin and returns how many samples it passed on.
  func pump() throws -> Int {
    defer { continuation.finish() }
    let stdin = FileHandle.standardInput
    var pending = Data()
    var total = 0
    while true {
      let chunk = stdin.availableData
      if chunk.isEmpty { break }
      pending.append(chunk)
      let usable = pending.count & ~1
      guard usable >= 3_200 else { continue }
      try send(pending.prefix(usable))
      total += usable / 2
      pending.removeFirst(usable)
    }
    let usable = pending.count & ~1
    if usable > 0 {
      try send(pending.prefix(usable))
      total += usable / 2
    }
    return total
  }

  private func send(_ bytes: Data) throws {
    let frames = AVAudioFrameCount(bytes.count / 2)
    guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames) else {
      throw HelperError("couldn't allocate an audio buffer")
    }
    buffer.frameLength = frames
    bytes.withUnsafeBytes { raw in
      let source = raw.bindMemory(to: Int16.self)
      let channel = buffer.int16ChannelData![0]
      for i in 0..<Int(frames) { channel[i] = Int16(littleEndian: source[i]) }
    }
    continuation.yield(AnalyzerInput(buffer: try convert(buffer)))
  }

  private func convert(_ buffer: AVAudioPCMBuffer) throws -> AVAudioPCMBuffer {
    guard let converter else { return buffer }
    let ratio = output.sampleRate / format.sampleRate
    let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 32
    guard let out = AVAudioPCMBuffer(pcmFormat: output, frameCapacity: capacity) else {
      throw HelperError("couldn't allocate an audio buffer")
    }
    var given = false
    var error: NSError?
    converter.convert(to: out, error: &error) { _, status in
      if given {
        status.pointee = .noDataNow
        return nil
      }
      given = true
      status.pointee = .haveData
      return buffer
    }
    if let error { throw error }
    return out
  }
}
#endif

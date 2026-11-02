@preconcurrency import AVFoundation
import Speech

/// One continuous recognizer receives the recording's exact 16 kHz mono Int16 PCM.
/// Only natural finalized results are emitted; Apple may defer them until EOF.
/// Times use the source clock, not a promise of batch lexical/alignment identity.
/// No model downloads. Any failure leaves Rust to transcribe the whole saved WAV.
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
    guard let choice = await SpeechModel.choose(preferred: preferred) else {
      throw HelperError("no speech model is available on this Mac")
    }
    let module = choice.module()
    guard await AssetInventory.status(forModules: [module]) == .installed else {
      throw HelperError("the speech model for \(choice.locale.identifier) isn't installed")
    }
    let pcm = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16000, channels: 1, interleaved: false)!
    let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [module]) ?? pcm
    let analyzer = SpeechAnalyzer(modules: [module], options: SpeechAnalyzer.Options(priority: .userInitiated, modelRetention: .whileInUse))
    // Independent buffers, bounded even while native preparation/consumption stalls.
    // Refuse overflow rather than drop audio or retain the entire recording.
    let (inputs, input) = AsyncThrowingStream<AnalyzerInput, Error>.makeStream(bufferingPolicy: .bufferingOldest(32))
    let (sourceEnd, sourceFinished) = AsyncThrowingStream<Int, Error>.makeStream()
    let (events, completion) = AsyncThrowingStream<LiveProgress, Error>.makeStream()
    let results = Task {
      do {
        func collect(_ attributed: AttributedString, _ range: CMTimeRange) throws {
          let text = String(attributed.characters).trimmingCharacters(in: .whitespacesAndNewlines)
          guard !text.isEmpty else { return }
          let times = try sourceTimes(range)
          var words: [[String: Any]] = []
          for run in attributed.runs {
            guard let local = run.audioTimeRange else { continue }
            let word = String(attributed[run.range].characters).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !word.isEmpty else { continue }
            let wordTimes = try sourceTimes(local)
            words.append(["t0": wordTimes.0, "t1": wordTimes.1, "text": word])
          }
          emit(["segment": ["t0": times.0, "t1": times.1, "text": text], "words": words])
        }
        if let transcriber = module as? SpeechTranscriber {
          for try await result in transcriber.results { try collect(result.text, result.range) }
        } else if let transcriber = module as? DictationTranscriber {
          for try await result in transcriber.results where result.isFinal { try collect(result.text, result.range) }
        } else { throw HelperError("unsupported live speech module") }
        completion.yield(.resultsDrained)
      } catch { completion.finish(throwing: error) }
    }
    let reader = Task.detached {
      do {
        let source = SourcePCM(format: pcm) { count in
          try FileHandle.standardInput.read(upToCount: count) ?? Data()
        }
        try source.convert(to: format) { buffer in
          guard case .enqueued = input.yield(AnalyzerInput(buffer: buffer)) else {
            throw HelperError("live audio input closed or budget exceeded")
          }
        }
        try Task.checkCancellation()
        input.finish()
        sourceFinished.yield(source.total); sourceFinished.finish()
        completion.yield(.samples(source.total))
      } catch {
        input.finish(throwing: error); sourceFinished.finish(throwing: error)
        completion.finish(throwing: error)
      }
    }
    let analysis = Task {
      do {
        try await analyzer.prepareToAnalyze(in: format)
        try Task.checkCancellation()
        emit(["ready": true, "locale": choice.locale.identifier, "engine": choice.name])
        // start attaches the sequence; its return does NOT mean input was drained.
        try await analyzer.start(inputSequence: inputs)
        var ended = false
        for try await _ in sourceEnd { ended = true }
        guard ended else { throw HelperError("live source did not finish") }
        try Task.checkCancellation()
        try await analyzer.finalizeAndFinishThroughEndOfInput()
        completion.yield(.terminalFinished)
      } catch { completion.finish(throwing: error) }
    }
    defer {
      input.finish(); sourceFinished.finish(); completion.finish()
      reader.cancel(); results.cancel(); analysis.cancel()
    }
    do {
      let total = try await finishLiveInput(events)
      emit(["done": true, "samples": total])
    } catch {
      // Never join a blocked read or uncooperative native task. Rust owns the hard
      // process-group cleanup/deadline; native cancellation is best effort only.
      Task { await analyzer.cancelAndFinishNow() }
      throw error
    }
  }

  private static func sourceTimes(_ range: CMTimeRange) throws -> (Int, Int) {
    let end = CMTimeRangeGetEnd(range)
    guard range.isValid, range.start.isNumeric, end.isNumeric,
          CMTimeCompare(range.start, .zero) >= 0, CMTimeCompare(end, range.start) >= 0 else {
      throw HelperError("invalid source speech range")
    }
    let startMs = (range.start.seconds * 1000).rounded()
    let endMs = (end.seconds * 1000).rounded()
    guard startMs.isFinite, endMs.isFinite, startMs >= 0, endMs < Double(Int.max) else {
      throw HelperError("invalid absolute speech range")
    }
    // Rust additionally checks every range against the finished WAV duration.
    return (Int(startMs), Int(endMs))
  }
  #endif
}

#if compiler(>=6.2)
private enum LiveProgress { case samples(Int), resultsDrained, terminalFinished }

/// Input success, terminal success AND complete result consumption are independent.
/// Any failure bypasses a blocked reader/preparation without awaiting either task.
private func finishLiveInput(_ events: AsyncThrowingStream<LiveProgress, Error>) async throws -> Int {
  try Task.checkCancellation()
  var total: Int?
  var drained = false; var finished = false
  for try await event in events {
    try Task.checkCancellation()
    switch event {
    case .samples(let count): total = count
    case .resultsDrained: drained = true
    case .terminalFinished: finished = true
    }
    if let total, drained && finished { return total }
  }
  try Task.checkCancellation()
  throw HelperError("live transcription ended without input, terminal and results drain")
}

/// Pull real source PCM through ONE converter for the whole recording. A short
/// pipe read is not EOF; a split Int16 survives until its next byte. No padding,
/// reset, finite conversion per read, guessed output count, or retained recording.
private final class SourcePCM {
  let format: AVAudioFormat
  let read: (Int) throws -> Data
  private var pending = Data()
  private var eof = false
  private(set) var total = 0

  init(format: AVAudioFormat, read: @escaping (Int) throws -> Data) {
    self.format = format; self.read = read
  }

  private func next(_ requested: Int) throws -> AVAudioPCMBuffer? {
    try Task.checkCancellation()
    guard requested > 0 else { throw HelperError("invalid converter request") }
    while pending.count < 2 && !eof {
      let bytes = try read(1600)
      guard bytes.count <= 1600 else { throw HelperError("source read exceeded budget") }
      eof = bytes.isEmpty
      pending.append(bytes)
    }
    if eof && pending.count % 2 != 0 { throw HelperError("incomplete PCM sample at EOF") }
    let count = min(requested, pending.count / 2)
    guard count > 0 else { return nil }
    guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(count)),
          let channel = buffer.int16ChannelData else { throw HelperError("can't allocate source PCM") }
    buffer.frameLength = AVAudioFrameCount(count)
    for i in 0..<count {
      let at = pending.startIndex + i * 2
      channel[0][i] = Int16(bitPattern: UInt16(pending[at]) | UInt16(pending[at + 1]) << 8)
    }
    pending.removeFirst(count * 2)
    let (next, overflow) = total.addingReportingOverflow(count)
    guard !overflow else { throw HelperError("live source clock overflow") }
    total = next
    return buffer
  }

  func convert(to output: AVAudioFormat, yield: (AVAudioPCMBuffer) throws -> Void) throws {
    if format == output {
      while let buffer = try next(800) { try yield(buffer) }
      try Task.checkCancellation()
      return
    }
    guard let converter = AVAudioConverter(from: format, to: output) else { throw HelperError("can't convert live PCM") }
    var stalled = 0
    var suppliedEOS = false
    while true {
      try Task.checkCancellation()
      guard let out = AVAudioPCMBuffer(pcmFormat: output, frameCapacity: 4096) else {
        throw HelperError("can't allocate converted PCM")
      }
      let before = total
      var failure: Error?
      var error: NSError?
      var callbacks = 0
      let status = converter.convert(to: out, error: &error) { requested, state in
        callbacks += 1
        if callbacks > 4096 || Task.isCancelled {
          failure = HelperError("converter callback failed or cancelled")
          state.pointee = .endOfStream; return nil
        }
        do {
          if let input = try self.next(Int(requested)) {
            state.pointee = .haveData; return input
          }
          suppliedEOS = true; state.pointee = .endOfStream; return nil
        } catch { failure = error; state.pointee = .endOfStream; return nil }
      }
      if let error { throw error }
      if let failure { throw failure }
      switch status {
      case .error: throw HelperError("converter error without NSError")
      case .haveData, .inputRanDry, .endOfStream: break
      @unknown default: throw HelperError("unknown converter output status")
      }
      if out.frameLength > 0 { try yield(out) }
      if status == .endOfStream {
        guard suppliedEOS && eof && pending.isEmpty else { throw HelperError("premature converter EOS") }
        try Task.checkCancellation()
        return
      }
      // Zero-output priming is legal. A wedged converter is not.
      stalled = total == before && out.frameLength == 0 ? stalled + 1 : 0
      if stalled >= 8 { throw HelperError("converter made no progress through EOS") }
    }
  }
}
#endif
// Appended to production LiveTranscribe.swift to exercise its private seams.
import Darwin
struct HelperError: LocalizedError {
  let errorDescription: String?
  init(_ message: String) { errorDescription = message }
}
func emit(_ object: Any) {}
func require(_ value: Bool, _ message: String) throws {
  if !value { throw HelperError(message) }
}
#if compiler(>=6.2)
func pcm(_ samples: [Int16]) -> Data {
  Data(samples.flatMap { v -> [UInt8] in
    let b = UInt16(bitPattern: v); return [UInt8(b & 255), UInt8(b >> 8)]
  })
}
func mustFail(_ body: () throws -> Void) throws {
  do { try body() } catch { return }
  throw HelperError("expected refusal")
}
actor CompletionFlag {
  var value = false
  func mark() { value = true }
}
// A third next() proves the coordinator processed the two available events
// and is waiting for the missing owner. An early-return bug never reaches it.
private struct ObservedProgress: AsyncSequence {
  typealias Element = LiveProgress
  let base: AsyncThrowingStream<LiveProgress, Error>
  let suspended: AsyncStream<Void>.Continuation
  struct AsyncIterator: AsyncIteratorProtocol {
    var base: AsyncThrowingStream<LiveProgress, Error>.Iterator
    let suspended: AsyncStream<Void>.Continuation
    var consumed = 0
    mutating func next() async throws -> LiveProgress? {
      if consumed == 2 { suspended.yield(()) }
      let value = try await base.next()
      if value != nil { consumed += 1 }
      return value
    }
  }
  func makeAsyncIterator() -> AsyncIterator {
    AsyncIterator(base: base.makeAsyncIterator(), suspended: suspended)
  }
}
extension LiveTranscriber {
  static func checkSourceTimes() throws {
    let times = try sourceTimes(CMTimeRange(start: CMTime(value: 7, timescale: 16000), duration: CMTime(value: 9, timescale: 16000)))
    try require(times.0 == 0 && times.1 == 1, "source clock rounding changed")
    try mustFail { _ = try sourceTimes(CMTimeRange(start: CMTime(value: -1, timescale: 16000), duration: .zero)) }
    try mustFail { _ = try sourceTimes(.invalid) }
  }
}
func identity(_ bytes: Data, sizes: [Int], format: AVAudioFormat) throws -> (Data, Int) {
  var offset = 0; var i = 0; var delivered = Data(); var lastYieldRead = 0
  let source = SourcePCM(format: format) { maximum in
    // A production pull must deliver incrementally, not accumulate a whole recording.
    try require(offset - lastYieldRead <= 1601, "source retained audio instead of streaming")
    let end = min(bytes.count, offset + min(maximum, sizes[i % sizes.count]))
    let part = bytes.subdata(in: offset..<end)
    offset = end; i += 1; return part
  }
  try source.convert(to: format) { buffer in
    lastYieldRead = offset
    guard let p = buffer.int16ChannelData else { throw HelperError("missing source channel") }
    delivered.append(pcm(Array(UnsafeBufferPointer(start: p[0], count: Int(buffer.frameLength)))))
  }
  return (delivered, source.total)
}
func naturalTests() async throws {
  try LiveTranscriber.checkSourceTimes()
  let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16000, channels: 1, interleaved: false)!
  for n in [0, 1, 159, 799, 800, 801, 32800, 32801] {
    let bytes = pcm((0..<n).map { Int16(truncatingIfNeeded: $0 * 197) })
    for sizes in [[1], [159, 1600, 999, 1], [1600]] {
      let (actual, count) = try identity(bytes, sizes: sizes, format: format)
      try require(actual == bytes && count == n, "odd seam/partial tail changed source bytes/count")
    }
  }
  try mustFail { _ = try identity(Data([1]), sizes: [1], format: format) }
  let failedSource = SourcePCM(format: format) { _ in throw HelperError("read failure") }
  try mustFail { try failedSource.convert(to: format) { _ in } }
  var reads = 0
  let refused = SourcePCM(format: format) { _ in reads += 1; return Data(repeating: 0, count: 1600) }
  try mustFail { try refused.convert(to: format) { _ in throw HelperError("consumer refused") } }
  try require(reads == 1, "read past failed delivery")
  // Actual completion seam: hold each owner independently. start/terminal return
  // cannot masquerade as source completion or result consumption.
  for missing in 0..<3 {
    let flag = CompletionFlag()
    let (events, input) = AsyncThrowingStream<LiveProgress, Error>.makeStream()
    let (waiting, suspended) = AsyncStream<Void>.makeStream()
    let observed = ObservedProgress(base: events, suspended: suspended)
    let worker = Task { let n = try await finishLiveInput(observed); await flag.mark(); return n }
    let values: [LiveProgress] = [.samples(32801), .terminalFinished, .resultsDrained]
    for i in 0..<3 where i != missing { input.yield(values[i]) }
    for await _ in waiting { break }
    suspended.finish()
    let premature = await flag.value
    try require(!premature, "done before all three owners succeeded")
    input.yield(values[missing]); input.finish()
    let count = try await worker.value
    try require(count == 32801, "source completion count changed")
  }
  for partial in [[LiveProgress](), [.samples(1)], [.terminalFinished, .resultsDrained]] {
    let (events, input) = AsyncThrowingStream<LiveProgress, Error>.makeStream()
    for value in partial { input.yield(value) }
    input.finish(throwing: HelperError("independent worker failed"))
    do { _ = try await finishLiveInput(events); throw HelperError("failure accepted") }
    catch { try require(error.localizedDescription == "independent worker failed", "worker error lost") }
  }
  let (cancelled, cancelInput) = AsyncThrowingStream<LiveProgress, Error>.makeStream()
  let cancelledOwner = Task { try await finishLiveInput(cancelled) }
  cancelledOwner.cancel(); cancelInput.finish()
  do { _ = try await cancelledOwner.value; throw HelperError("cancel accepted") }
  catch { try require(error is CancellationError, "cancel became success") }
  // Resampling with fragmented source reads must preserve terminal samples and
  // match an independently fed converter drained through real EOS.
  let output = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48000, channels: 1, interleaved: false)!
  for n in [1, 799, 800, 801, 32801] {
    let values = (0..<n).map { Int16(truncatingIfNeeded: $0 * 197) }
    let bytes = pcm(values)
    var at = 0
    let source = SourcePCM(format: format) { maximum in
      let end = min(bytes.count, at + min(maximum, 159))
      defer { at = end }
      return bytes.subdata(in: at..<end)
    }
    var actual = Data()
    try source.convert(to: output) { buffer in
      actual.append(Data(bytes: buffer.floatChannelData![0], count: Int(buffer.frameLength) * 4))
    }
    let converter = AVAudioConverter(from: format, to: output)!
    var fed = 0; var eof = false; var expected = Data(); var ended = false
    for _ in 0..<512 {
      let buffer = AVAudioPCMBuffer(pcmFormat: output, frameCapacity: 4096)!
      var error: NSError?
      let status = converter.convert(to: buffer, error: &error) { requested, state in
        if fed == n { eof = true; state.pointee = .endOfStream; return nil }
        let count = min(Int(requested), n - fed)
        let input = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(count))!
        input.frameLength = AVAudioFrameCount(count)
        for i in 0..<count { input.int16ChannelData![0][i] = values[fed + i] }
        fed += count; state.pointee = .haveData; return input
      }
      if let error { throw error }
      try require(status != .error, "reference converter failed")
      expected.append(Data(bytes: buffer.floatChannelData![0], count: Int(buffer.frameLength) * 4))
      if status == .endOfStream { ended = true; break }
    }
    try require(ended && eof && fed == n, "reference did not reach source EOS")
    try require(source.total == n && actual == expected, "fragmented conversion lost or changed source tail")
  }
}

#endif
@main struct NaturalTests {
  static func main() async {
    do {
      #if compiler(>=6.2)
      try await naturalTests(); print("actual production source/completion seams passed"); return
      #else
      throw HelperError("Swift 6.2 required")
      #endif
    } catch { fputs("FAILED: \(error)\n", stderr); exit(1) }
  }
}

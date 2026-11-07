import CoreAudio
import Foundation
import Darwin

// All channels are read before the first write, and verified as one group. Never
// snapshot or restore a channel independently: driver writes can affect its peers.
protocol PlaybackOutput: AnyObject {
  func read() -> [Float]?
  func write(_ values: [Float]) -> Bool
}

final class CorePlaybackOutput: PlaybackOutput {
  let device: AudioDeviceID
  let elements: [UInt32]

  static func defaultDevice() -> AudioDeviceID? {
    var device = AudioDeviceID(0)
    var bytes = UInt32(MemoryLayout<AudioDeviceID>.size)
    var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultOutputDevice,
      mScope: kAudioObjectPropertyScopeGlobal, mElement: 0)
    guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &bytes, &device) == noErr,
          device != 0 else { return nil }
    return device
  }

  static func current() -> CorePlaybackOutput? {
    guard let device = defaultDevice() else { return nil }
    var bytes: UInt32 = 0
    var streams = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreamConfiguration,
      mScope: kAudioDevicePropertyScopeOutput, mElement: 0)
    bytes = 0
    guard AudioObjectGetPropertyDataSize(device, &streams, 0, nil, &bytes) == noErr, bytes > 0 else { return nil }
    let storage = UnsafeMutableRawPointer.allocate(byteCount: Int(bytes), alignment: MemoryLayout<AudioBufferList>.alignment)
    defer { storage.deallocate() }
    guard AudioObjectGetPropertyData(device, &streams, 0, nil, &bytes, storage) == noErr else { return nil }
    let buffers = UnsafeMutableAudioBufferListPointer(storage.assumingMemoryBound(to: AudioBufferList.self))
    let count = buffers.reduce(UInt32(0)) { $0 + $1.mNumberChannels }
    let channels = count > 0 ? Array(1...count) : []
    let separate = CorePlaybackOutput(device: device, elements: channels)
    if !channels.isEmpty, separate.canWrite(), separate.read() != nil { return separate }
    // Do not use master volume if it could overwrite an observable channel balance
    // that cannot be restored individually.
    if channels.contains(where: { CorePlaybackOutput(device: device, elements: [$0]).read() != nil }) { return nil }
    let master = CorePlaybackOutput(device: device, elements: [0])
    return master.canWrite() && master.read() != nil ? master : nil
  }

  init(device: AudioDeviceID, elements: [UInt32]) {
    self.device = device
    self.elements = elements
  }

  func property(_ element: UInt32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyVolumeScalar,
      mScope: kAudioDevicePropertyScopeOutput, mElement: element)
  }

  func canWrite() -> Bool {
    elements.allSatisfy {
      var address = property($0)
      var settable = DarwinBoolean(false)
      return AudioObjectIsPropertySettable(device, &address, &settable) == noErr && settable.boolValue
    }
  }

  func read() -> [Float]? {
    var values: [Float] = []
    for element in elements {
      var address = property(element)
      var value: Float = 0
      var bytes = UInt32(MemoryLayout<Float>.size)
      guard AudioObjectGetPropertyData(device, &address, 0, nil, &bytes, &value) == noErr,
            value.isFinite, (0...1).contains(value) else { return nil }
      values.append(value)
    }
    return values
  }

  func write(_ values: [Float]) -> Bool {
    guard values.count == elements.count else { return false }
    var success = true
    for (element, value) in zip(elements, values) {
      var address = property(element)
      var scalar = value
      if AudioObjectSetPropertyData(device, &address, 0, nil, UInt32(MemoryLayout<Float>.size), &scalar) != noErr {
        success = false
      }
    }
    return success
  }
}

final class PlaybackRamp {
  let output: PlaybackOutput
  let original: [Float]
  var accepted: [Float]
  var from: [Float]
  var target: [Float]
  var started: Double
  var restoring = false
  var finished = false
  var settled = false

  init?(output: PlaybackOutput, now: Double) {
    guard let values = output.read(), !values.isEmpty else { return nil }
    self.output = output
    original = values
    accepted = values
    from = values
    target = values
    started = now
  }

  func owned() -> Bool {
    guard let current = output.read(), current.count == accepted.count else { return false }
    return zip(current, accepted).allSatisfy { abs($0 - $1) < 0.002 }
  }

  func retarget(_ factor: Float?, now: Double) {
    // During a ramp Bluetooth may still report an earlier channel write. Only
    // infer manual changes after the target has settled, never mid-transition.
    guard !finished, !settled || owned() else { finished = true; return }
    from = output.read() ?? accepted
    target = factor.map { factor in original.map { $0 * factor } } ?? original
    restoring = factor == nil
    settled = false
    started = now
  }

  func tick(now: Double) {
    if settled { return }
    guard !finished else { return }
    let t = Float(min(1, max(0, now - started)))
    let eased = t * t * (3 - 2 * t)
    let values = zip(from, target).map { $0 + ($1 - $0) * eased }
    if !output.write(values) {
      // Roll back the entire snapshot, including peers of a failed channel.
      _ = output.write(original)
      finished = true
      return
    }
    guard let actual = output.read() else {
      _ = output.write(original)
      finished = true
      return
    }
    accepted = actual // Hardware is allowed to quantize its scalar values.
    if t >= 1 {
      // Keep the final command in flight until every channel acknowledges it.
      // An immediate read can contain one old and one new Bluetooth value.
      let acknowledged = zip(actual, target).allSatisfy { abs($0 - $1) < 0.002 }
      if acknowledged || now - started >= 1.5 {
        settled = true
        if restoring { finished = true }
      }
    }
  }
}

// Written to from signal handlers, which can't capture anything: wakes the loop below.
private var signalPipe: [Int32] = [-1, -1]

func runPlaybackVolume(arguments: [String]) {
  if arguments.first == "--self-test" { testPlaybackRamp(); return }
  if arguments.first == "--inspect" {
    if let output = CorePlaybackOutput.current(), let values = output.read() {
      emit(["device": output.device, "elements": output.elements, "volumes": values])
    } else { emit(["supported": false]) }
    return
  }
  var cachedOutput = CorePlaybackOutput.current()
  var ramp: PlaybackRamp?
  var disconnected = false
  var pending = ""
  func now() -> Double { ProcessInfo.processInfo.systemUptime }
  func start(_ percent: Float) {
    if ramp == nil || ramp!.finished {
      if cachedOutput?.device != CorePlaybackOutput.defaultDevice() {
        cachedOutput = CorePlaybackOutput.current()
      }
      ramp = cachedOutput.flatMap { PlaybackRamp(output: $0, now: now()) }
    }
    ramp?.retarget(percent / 100, now: now())
  }
  if let raw = arguments.first, let percent = Float(raw), (0...100).contains(percent) { start(percent) }
  // Terminated (`pkill`, an installer, logout) rather than told to stop: put the volume back at
  // once, without a ramp, unless someone changed it since.
  _ = pipe(&signalPipe)
  for sig in [SIGTERM, SIGINT, SIGHUP] {
    signal(sig) { _ in
      var byte: UInt8 = 1
      _ = Darwin.write(signalPipe[1], &byte, 1)
    }
  }
  func terminate() -> Never {
    if let ramp, !ramp.finished, !ramp.settled || ramp.owned() { _ = ramp.output.write(ramp.original) }
    exit(0)
  }
  if arguments.first == "--server" { emit(["event": "ready"]) }
  // A warm helper waits on the pipe. The first volume step runs within one 16 ms
  // tick of a start command, independently of screen/microphone initialization.
  while true {
    var descriptors = [
      pollfd(fd: disconnected ? -1 : STDIN_FILENO, events: Int16(POLLIN | POLLHUP), revents: 0),
      pollfd(fd: signalPipe[0], events: Int16(POLLIN), revents: 0),
    ]
    let timeout: Int32 = !disconnected && (ramp == nil || ramp?.settled == true) ? -1 : 16
    let ready = poll(&descriptors, 2, timeout)
    if descriptors[1].revents != 0 { terminate() }
    if !disconnected {
      if ready > 0 && descriptors[0].revents != 0 {
        var buffer = [UInt8](repeating: 0, count: 1024)
        let count = Darwin.read(STDIN_FILENO, &buffer, buffer.count)
        if count <= 0 {
          disconnected = true
          ramp?.retarget(nil, now: now())
        } else {
          pending += String(decoding: buffer.prefix(count), as: UTF8.self)
          while let newline = pending.firstIndex(of: "\n") {
            let line = String(pending[..<newline])
            pending.removeSubrange(...newline)
            if line == "stop" { ramp?.retarget(nil, now: now()) }
            else if line.hasPrefix("start "), let percent = Float(line.dropFirst(6)), (0...100).contains(percent) { start(percent) }
          }
        }
      }
    }
    ramp?.tick(now: now())
    if ramp?.finished == true { ramp = nil }
    if disconnected && ramp == nil { break }
  }
}

// Deterministic tests use synthetic controls, never speaker playback or a mic.
func testPlaybackRamp() {
  final class Fake: PlaybackOutput {
    var values: [Float]
    var failNext = false
    init(_ values: [Float]) { self.values = values }
    func read() -> [Float]? { values }
    func write(_ incoming: [Float]) -> Bool {
      if failNext { failNext = false; values[0] = incoming[0]; return false }
      values = incoming.map { ($0 * 1000).rounded() / 1000 }
      return true
    }
  }
  func close(_ values: [Float], _ expected: [Float]) -> Bool {
    zip(values, expected).allSatisfy { abs($0 - $1) < 0.002 }
  }
  for original: [Float] in [[0.3, 0.8], [0, 0.8], [0.5, 0.5], [0.4]] {
    let output = Fake(original)
    let ramp = PlaybackRamp(output: output, now: 0)!
    ramp.retarget(0, now: 0)
    ramp.tick(now: 0.5)
    precondition(close(output.values, original.map { $0 / 2 }))
    ramp.tick(now: 1)
    precondition(close(output.values, original.map { _ in 0 }))
    ramp.retarget(nil, now: 2)
    ramp.tick(now: 2.5)
    precondition(close(output.values, original.map { $0 / 2 }))
    ramp.tick(now: 3)
    precondition(close(output.values, original) && ramp.finished)
  }
  let output = Fake([0.3, 0.8])
  let ramp = PlaybackRamp(output: output, now: 0)!
  ramp.retarget(0.2, now: 0)
  ramp.tick(now: 0.5)
  let partial = output.values
  ramp.retarget(nil, now: 0.5)
  ramp.tick(now: 0.5)
  precondition(close(output.values, partial)) // Early stop has no jump.
  ramp.tick(now: 1)
  let returning = output.values
  ramp.retarget(0, now: 1)
  ramp.tick(now: 1)
  precondition(close(output.values, returning)) // Restart reverses smoothly.
  ramp.tick(now: 2)
  ramp.retarget(nil, now: 2)
  ramp.tick(now: 3)
  precondition(close(output.values, [0.3, 0.8]))
  let manual = Fake([0.3, 0.8])
  let changed = PlaybackRamp(output: manual, now: 0)!
  changed.retarget(0.2, now: 0)
  changed.tick(now: 1)
  manual.values = [0.5, 0.5]
  changed.retarget(nil, now: 2)
  changed.tick(now: 3)
  precondition(manual.values == [0.5, 0.5])
  let failed = Fake([0.3, 0.8])
  let recovery = PlaybackRamp(output: failed, now: 0)!
  recovery.retarget(0.2, now: 0)
  failed.failNext = true
  recovery.tick(now: 0.5)
  precondition(close(failed.values, [0.3, 0.8]) && recovery.finished)
  final class Delayed: PlaybackOutput {
    var values: [Float] = [0.3, 0.8]
    var pending: [Float] = [0.3, 0.8]
    var next = 0
    func read() -> [Float]? {
      // Each query acknowledges only one channel of a previous write.
      values[next] = pending[next]
      next = 1 - next
      return values
    }
    func write(_ values: [Float]) -> Bool { pending = values; return true }
  }
  let delayed = Delayed()
  let asynchronous = PlaybackRamp(output: delayed, now: 0)!
  asynchronous.retarget(0, now: 0)
  for step in 0...130 { asynchronous.tick(now: Double(step) / 100) }
  precondition(asynchronous.settled && !asynchronous.finished)
  asynchronous.retarget(nil, now: 2)
  for step in 200...330 { asynchronous.tick(now: Double(step) / 100) }
  precondition(asynchronous.finished && close(delayed.read()!, [0.3, 0.8]))
  print("Playback ramp tests passed (stereo, mute, restore, reversal, manual override, partial failure, delayed channel feedback).")
}

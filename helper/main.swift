// scrawl-helper: the parts of Scrawl that need Swift-only Apple frameworks.
//
//   scrawl-helper devices                               cabled iPhones/iPads (USB registry) as JSON
//   scrawl-helper mirror --out FILE.mov [--device ID] [--microphone UID --microphone-name NAME]
//                                                       live mirror window + native recording (JSON lines)
//   scrawl-helper transcribe FILE [--locale ID]          SpeechAnalyzer transcript with word timings
//   scrawl-helper transcribe-stream [--locale ID]        the same while recording: PCM on stdin, JSON lines (LiveTranscribe.swift)
//   scrawl-helper speech-model [--install] [--locale ID] the speech model's status; install it (JSON lines)
//   scrawl-helper frames MOVIE --out-dir DIR (--every MS | --at MS,MS…) [--max-edge PX]
//   scrawl-helper audio MOVIE OUT.wav                   16 kHz mono PCM of every audio track, mixed
//   scrawl-helper probe MOVIE                           duration and video size
//   scrawl-helper shrink MOVIE --out FILE.mov [--max-edge PX] [--bitrate BPS]   HEVC copy, same timeline
//   scrawl-helper export PLAN.json                      shareable .mp4: crop/camera, burned-in ink, audio (Export.swift)
//   scrawl-helper permissions                           what Scrawl has been granted, as JSON
//   scrawl-helper sck-probe [--watch SECONDS]           can this process capture with ScreenCaptureKit (ScreenProbe.swift)
//   scrawl-helper qr TEXT                               half-block QR code for `scrawl link`
//
// Every timestamp in the output is integer milliseconds. Scrawl drives the long-running
// `mirror` command over stdin (`stop`, `cancel`) and reads its events from stdout.

import AppKit
import CoreImage
import Foundation

func renderQR(_ message: String) -> String? {
  guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
  filter.setValue(Data(message.utf8), forKey: "inputMessage")
  filter.setValue("L", forKey: "inputCorrectionLevel")
  guard let image = filter.outputImage else { return nil }
  let extent = image.extent.integral
  let width = Int(extent.width)
  let height = Int(extent.height)
  guard width > 0, height > 0 else { return nil }
  var pixels = [UInt8](repeating: 255, count: width * height * 4)
  let context = CIContext(options: [.useSoftwareRenderer: true])
  context.render(
    image,
    toBitmap: &pixels,
    rowBytes: width * 4,
    bounds: extent,
    format: .RGBA8,
    colorSpace: CGColorSpaceCreateDeviceRGB()
  )
  func isDark(_ x: Int, _ y: Int) -> Bool {
    guard x >= 0, x < width, y >= 0, y < height else { return false }
    // CIImage origin is bottom-left; flip y so top row renders first.
    let flippedY = (height - 1) - y
    return pixels[(flippedY * width + x) * 4] < 128
  }
  let pad = 1
  var lines: [String] = []
  var y = -pad
  while y < height + pad {
    var line = ""
    for x in -pad ..< (width + pad) {
      let top = isDark(x, y)
      let bottom = (y + 1 < height + pad) ? isDark(x, y + 1) : false
      switch (top, bottom) {
      case (true, true): line.append("█")
      case (true, false): line.append("▀")
      case (false, true): line.append("▄")
      case (false, false): line.append(" ")
      }
    }
    lines.append(line)
    y += 2
  }
  return lines.joined(separator: "\n")
}

func emit(_ object: Any) {
  guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes]) else { return }
  FileHandle.standardOutput.write(data + Data("\n".utf8))
}

func fail(_ message: String) -> Never {
  FileHandle.standardError.write(Data("scrawl-helper: \(message)\n".utf8))
  exit(1)
}

struct HelperError: LocalizedError {
  let errorDescription: String?
  init(_ message: String) { errorDescription = message }
}

func option(_ name: String, in arguments: [String]) -> String? {
  guard let index = arguments.firstIndex(of: name), index + 1 < arguments.count else { return nil }
  return arguments[index + 1]
}

/// Runs async work from a synchronous entry point, keeping the main queue free for AVFoundation.
func runAsync(_ body: @escaping () async throws -> Void) -> Never {
  Task {
    do {
      try await body()
      exit(0)
    } catch {
      fail(error.localizedDescription)
    }
  }
  dispatchMain()
}

let arguments = Array(CommandLine.arguments.dropFirst())
switch arguments.first {
case "devices":
  emit(Phone.cabled())
case "mirror":
  let preview = arguments.contains("--preview")
  let hidden = arguments.contains("--hidden")
  let out = option("--out", in: arguments).map { URL(filePath: $0) }
  if !preview && out == nil { fail("mirror needs --out FILE.mov or --preview") }
  Mirror.run(
    output: out, previewOnly: preview, hiddenPreview: hidden, deviceID: option("--device", in: arguments),
    microphone: option("--microphone", in: arguments).map { ($0, option("--microphone-name", in: arguments) ?? $0) }
  )
case "transcribe":
  guard arguments.count >= 2 else { fail("transcribe needs a file") }
  runAsync { emit(try await Transcriber.transcribe(URL(filePath: arguments[1]), preferred: option("--locale", in: arguments))) }
case "transcribe-stream":
  runAsync { try await LiveTranscriber.run(preferred: option("--locale", in: arguments)) }
case "speech-model":
  let install = arguments.contains("--install")
  runAsync { await SpeechModel.run(install: install, preferred: option("--locale", in: arguments)) }
case "frames":
  guard arguments.count >= 2, let dir = option("--out-dir", in: arguments) else {
    fail("frames needs MOVIE --out-dir DIR (--every MS | --at MS,MS…)")
  }
  let maxEdge = option("--max-edge", in: arguments).flatMap(Int.init) ?? 0
  let every = option("--every", in: arguments).flatMap(Int.init)
  let at = option("--at", in: arguments)?.split(separator: ",").compactMap { Int($0) }
  runAsync {
    emit(try await Media.frames(movie: URL(filePath: arguments[1]), outDir: URL(filePath: dir), everyMs: every, atMs: at, maxEdge: maxEdge))
  }
case "audio":
  guard arguments.count >= 3 else { fail("audio needs MOVIE OUT.wav") }
  runAsync { emit(try await Media.exportAudio(movie: URL(filePath: arguments[1]), wav: URL(filePath: arguments[2]))) }
case "shrink":
  guard arguments.count >= 2, let out = option("--out", in: arguments) else { fail("shrink needs MOVIE --out FILE.mov") }
  let maxEdge = option("--max-edge", in: arguments).flatMap(Int.init) ?? 1600
  let bitRate = option("--bitrate", in: arguments).flatMap(Int.init) ?? 2_500_000
  runAsync {
    emit(try await Media.shrink(movie: URL(filePath: arguments[1]), out: URL(filePath: out), maxEdge: maxEdge, bitRate: bitRate))
  }
case "export":
  guard arguments.count >= 2 else { fail("export needs PLAN.json") }
  runAsync { try await Exporter.run(plan: URL(filePath: arguments[1])) }
case "probe":
  guard arguments.count >= 2 else { fail("probe needs MOVIE") }
  runAsync { emit(try await Media.probe(URL(filePath: arguments[1]))) }
case "permissions":
  emit(Permissions.statuses())
case "sck-probe":
  let watch = option("--watch", in: arguments).flatMap(Double.init)
  runAsync { await ScreenProbe.run(watch: watch) }
case "qr":
  guard arguments.count >= 2, let qr = renderQR(arguments[1]) else {
    fail("qr needs a text argument")
  }
  emit(["qr": qr])
default:
  fail("usage: scrawl-helper devices|mirror|transcribe|transcribe-stream|speech-model|frames|audio|probe|export|shrink|permissions|sck-probe|qr …")
}

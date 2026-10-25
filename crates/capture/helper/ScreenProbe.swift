import CoreGraphics
import Foundation
import ScreenCaptureKit

// `scrawl-helper sck-probe [--watch SECONDS]`: can this process capture the screen with
// ScreenCaptureKit right now? One JSON line per try:
//   {"ok":true,"preflight":false,"shareableContent":true,"screenshot":"64x40","ms":38,"pid":123}
// With --watch it tries once a second for that long without restarting, so it can be left
// running while Screen Recording is turned on to see whether a grant reaches a running process
// (CGPreflightScreenCaptureAccess keeps its first answer; the capture is the real test). It asks
// for Screen Recording if that was never decided. Which app the grant is checked against depends
// on who started it: Scrawl.app when Scrawl runs it, the terminal when you do.

enum ScreenProbe {
  static func once() async -> [String: Any] {
    let start = Date()
    var result: [String: Any] = [
      "preflight": CGPreflightScreenCaptureAccess(),
      "shareableContent": false,
      "pid": Int(getpid()),
    ]
    do {
      let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
      result["shareableContent"] = true
      guard let display = content.displays.first else { throw HelperError("no displays") }
      let filter = SCContentFilter(display: display, excludingWindows: [])
      let config = SCStreamConfiguration()
      config.width = 64
      config.height = 40
      let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
      result["screenshot"] = "\(image.width)x\(image.height)"
      result["ok"] = image.width > 0 && image.height > 0
    } catch {
      result["ok"] = false
      result["error"] = error.localizedDescription
    }
    result["ms"] = Int(Date().timeIntervalSince(start) * 1000)
    return result
  }

  static func run(watch: Double?) async {
    guard let watch else {
      emit(await once())
      return
    }
    let end = Date().addingTimeInterval(watch)
    repeat {
      emit(await once())
      try? await Task.sleep(nanoseconds: 1_000_000_000)
    } while Date() < end
  }
}

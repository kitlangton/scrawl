import AVFoundation
import ApplicationServices
import CoreGraphics
import Speech

// `scrawl-helper permissions`: what macOS has granted Scrawl.

enum Permissions {
  static func statuses() -> [String: String] {
    func av(_ type: AVMediaType) -> String {
      switch AVCaptureDevice.authorizationStatus(for: type) {
      case .authorized: return "granted"
      case .denied, .restricted: return "denied"
      default: return "not-determined"
      }
    }
    var statuses = [
      "screen-recording": CGPreflightScreenCaptureAccess() ? "granted" : "not-determined",
      "microphone": av(.audio),
      "accessibility": AXIsProcessTrusted() ? "granted" : "not-determined",
      "camera": av(.video),
    ]
    // Only SFSpeechRecognizer, the macOS 15 transcriber, needs it; SpeechAnalyzer doesn't.
    if #unavailable(macOS 26) {
      switch SFSpeechRecognizer.authorizationStatus() {
      case .authorized: statuses["speech-recognition"] = "granted"
      case .denied, .restricted: statuses["speech-recognition"] = "denied"
      default: statuses["speech-recognition"] = "not-determined"
      }
    }
    return statuses
  }

  static func request(_ permission: String) {
    switch permission {
    case "microphone": AVCaptureDevice.requestAccess(for: .audio) { _ in }
    case "camera": AVCaptureDevice.requestAccess(for: .video) { _ in }
    case "speech-recognition": SFSpeechRecognizer.requestAuthorization { _ in }
    case "accessibility":
      let prompt = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
      _ = AXIsProcessTrustedWithOptions([prompt: true] as CFDictionary)
    case "screen-recording": _ = CGRequestScreenCaptureAccess()
    default: break
    }
  }
}

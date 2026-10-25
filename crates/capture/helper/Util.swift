import Foundation

func log(_ message: String) {
  FileHandle.standardError.write(Data("scrawl-helper: \(message)\n".utf8))
}

func nowMs() -> Int {
  Int((Date().timeIntervalSince1970 * 1000).rounded())
}

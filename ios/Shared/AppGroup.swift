import Foundation
import os

/// The container the app and its extensions share: pairing, the destination and the captures.
enum AppGroup {
  /// `group.<app bundle ID>`, from each target's Info.plist (`ScrawlAppGroup`).
  static let identifier: String = {
    if let id = Bundle.main.object(forInfoDictionaryKey: "ScrawlAppGroup") as? String, !id.isEmpty, !id.contains("$(") {
      return id
    }
    return "group.dev.kitlangton.scrawl.ios"
  }()

  /// The shared container, or this process's own Application Support when there is none (a build
  /// without the entitlement), so nothing is lost; the extensions just can't see it.
  static let container: URL = {
    if let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier) {
      return url
    }
    log.error("No app group container for \(identifier, privacy: .public); using the process's own")
    return URL.applicationSupportDirectory.appending(path: "ScrawlGroup", directoryHint: .isDirectory)
  }()

  nonisolated(unsafe) static let defaults: UserDefaults = UserDefaults(suiteName: identifier) ?? .standard
}

let log = Logger(subsystem: "dev.kitlangton.scrawl.ios", category: Bundle.main.bundleURL.deletingPathExtension().lastPathComponent)

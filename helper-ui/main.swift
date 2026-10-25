// scrawl-ui-helper: Scrawl's SwiftUI onboarding, Settings, permission panel, and playback-volume helper.
//
//   scrawl-ui-helper permission-panel --permission P [--from X,Y,W,H]   drag-to-allow card (PermissionPanel.swift)
//   scrawl-ui-helper onboarding [--shortcut L] [--render DIR]           the welcome window (Onboarding.swift)
//   scrawl-ui-helper settings [--pane P] [--render DIR]                 the Settings window (Settings.swift)
//   scrawl-ui-helper playback-volume [--server | --restore]             CoreAudio output attenuation (PlaybackVolume.swift)

import AppKit
import Foundation

func emit(_ object: Any) {
  guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes]) else { return }
  FileHandle.standardOutput.write(data + Data("\n".utf8))
}

func fail(_ message: String) -> Never {
  FileHandle.standardError.write(Data("scrawl-ui-helper: \(message)\n".utf8))
  exit(1)
}

func option(_ name: String, in arguments: [String]) -> String? {
  guard let index = arguments.firstIndex(of: name), index + 1 < arguments.count else { return nil }
  return arguments[index + 1]
}

let arguments = Array(CommandLine.arguments.dropFirst())
switch arguments.first {
case "permission-panel":
  PermissionPanel.run(arguments: arguments)
case "onboarding":
  Onboarding.run(arguments: arguments)
case "playback-volume":
  runPlaybackVolume(arguments: Array(arguments.dropFirst()))
case "settings":
  SettingsWindow.run(arguments: arguments)
default:
  fail("usage: scrawl-ui-helper permission-panel|onboarding|settings|playback-volume …")
}

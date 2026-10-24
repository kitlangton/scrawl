import AppIntents

/// Flags this moment in the screen recording Scrawl is making (the cabled mirror's M key):
/// from the Action button, Back Tap, Siri or Shortcuts, without leaving the app being recorded.
struct FlagMomentIntent: AppIntent {
  static let title: LocalizedStringResource = "Flag Moment"
  static let description = IntentDescription("Flags this moment in the screen recording Scrawl is making. It gets its own keyframe, marked ⚑ for the agent.")
  static let openAppWhenRun = false

  enum FlagError: Error, CustomLocalizedStringResourceConvertible {
    case notRecording

    var localizedStringResource: LocalizedStringResource { "Scrawl isn't recording." }
  }

  func perform() async throws -> some IntentResult {
    guard ActiveBroadcast.current() != nil else { throw FlagError.notRecording }
    // Nothing shown: anything on screen would be in the recording.
    Ping.flag.post()
    return .result()
  }
}

struct ScrawlShortcuts: AppShortcutsProvider {
  static var appShortcuts: [AppShortcut] {
    AppShortcut(
      intent: FlagMomentIntent(),
      phrases: ["Flag this in \(.applicationName)", "\(.applicationName) flag"],
      shortTitle: "Flag Moment",
      systemImageName: "flag.fill"
    )
  }
}

import BackgroundTasks
import SwiftUI
import UserNotifications

@main
struct ScrawlApp: App {
  @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
  @State private var model = AppModel()
  @Environment(\.scenePhase) private var scenePhase

  var body: some Scene {
    WindowGroup {
      HomeView()
        .environment(model)
        .tint(.scrawlRed)
        .onOpenURL { model.open($0) }
        .onChange(of: scenePhase, initial: true) { _, phase in
          switch phase {
          case .active: Task { await model.refresh() }
          case .background: BackgroundSend.schedule()
          default: break
          }
        }
    }
  }
}

final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
  func application(_: UIApplication, didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
    UNUserNotificationCenter.current().delegate = self
    BackgroundSend.register()
    // A capture left waiting (an upload failed, even while the app is in the background) gets a
    // background retry.
    Retry.schedule = { BackgroundSend.schedule() }
    // Replies that came in while the app wasn't running, from its uploads and the extensions'.
    Uploader.shared.reconcile()
    return true
  }

  /// iOS finished (or gave up on) an upload while the app wasn't running, maybe one an
  /// extension started: hear how it went.
  func application(_: UIApplication, handleEventsForBackgroundURLSession identifier: String, completionHandler: @escaping () -> Void) {
    nonisolated(unsafe) let done = completionHandler
    if !Uploader.shared.handleEvents(for: identifier, completion: { done() }) { completionHandler() }
  }

  /// "Sent to …" shows as a banner even with Scrawl open.
  nonisolated func userNotificationCenter(_: UNUserNotificationCenter, willPresent _: UNNotification) async -> UNNotificationPresentationOptions {
    [.banner, .list]
  }
}

/// Sends what's waiting while Scrawl is in the background, when iOS lets it.
enum BackgroundSend {
  static let identifier = "\(Bundle.main.bundleIdentifier ?? "dev.kitlangton.scrawl.ios").send"

  static func register() {
    BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: nil) { task in
      nonisolated(unsafe) let task = task
      let work = Task { await Sender.sendDue(force: false) }
      task.expirationHandler = { work.cancel() }
      Task {
        await work.value
        schedule()
        task.setTaskCompleted(success: true)
      }
    }
  }

  static func schedule() {
    guard let due = Sender.nextDue() else { return }
    let request = BGAppRefreshTaskRequest(identifier: identifier)
    request.earliestBeginDate = max(due, Date().addingTimeInterval(60))
    try? BGTaskScheduler.shared.submit(request)
  }
}

extension Color {
  /// Scrawl's ink (`#ff3b30`).
  static let scrawlRed = Color(red: 1, green: 0.231, blue: 0.188)
}

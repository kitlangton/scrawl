import Foundation
import UserNotifications

/// Says what became of a capture, from whichever process heard: one notification per capture
/// (its ID), so "Sending to …" is replaced by how it went.
enum Notifier {
  static func requestAuthorization() async {
    _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
  }

  static func post(_ capture: Capture) {
    let content = UNMutableNotificationContent()
    content.title = capture.statusTitle
    content.body = [capture.lengthText, capture.statusDetail].compactMap(\.self).joined(separator: " · ")
    content.threadIdentifier = "captures"
    content.userInfo = ["capture": capture.id]
    switch capture.state {
    case .sending, .recording:
      // Shows without a sound; the outcome replaces it.
      content.interruptionLevel = .passive
    case .accepted(.delivered):
      content.sound = .default
    default:
      content.interruptionLevel = .active
    }
    let request = UNNotificationRequest(identifier: capture.id, content: content, trigger: nil)
    UNUserNotificationCenter.current().add(request)
  }
}

import XCTest

/// The whole way from this iPhone to an OpenCode session: pair with a Scrawl ingest server, pick
/// a session, send a test capture, and wait for the Mac to say it's queued in that session.
///
/// Runs only with a server to send to: `ios/scripts/e2e.sh` starts a sandboxed `scrawl serve`
/// (its own config, captures and port) with a stand-in OpenCode, and passes its setup link and
/// session title in `SCRAWL_E2E_LINK` and `SCRAWL_E2E_SESSION`.
final class EndToEndTests: XCTestCase {
  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  func testATestCaptureReachesTheSession() throws {
    let environment = ProcessInfo.processInfo.environment
    guard let link = environment["SCRAWL_E2E_LINK"], !link.isEmpty else {
      throw XCTSkip("Needs a Scrawl to send to: run ios/scripts/e2e.sh")
    }
    let session = environment["SCRAWL_E2E_SESSION"] ?? "E2E SESSION"

    let app = XCUIApplication()
    app.launchArguments = ["-scrawl-link", link]
    addUIInterruptionMonitor(withDescription: "Notifications") { alert in
      let allow = alert.buttons["Allow"]
      guard allow.exists else { return false }
      allow.tap()
      return true
    }
    app.launch()
    allowNotifications(app)

    let status = app.staticTexts["mac-status"]
    XCTAssert(status.waitForExistence(timeout: 20))
    XCTAssert(wait(for: status, labelBeginsWith: "Connected", timeout: 20), "Mac status: \(status.label)")
    attach(app, "1 Paired")

    app.buttons["destination"].tap()
    let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", session)).firstMatch
    XCTAssert(row.waitForExistence(timeout: 15), "The Mac's sessions should be listed")
    attach(app, "2 Sessions")
    row.tap()

    app.buttons["settings"].tap()
    app.buttons["send-test"].tap()
    attach(app, "3 Settings")
    app.navigationBars.buttons.element(boundBy: 0).tap()

    let sent = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "Sent to \(session)")).firstMatch
    XCTAssert(sent.waitForExistence(timeout: 120), "The capture should reach \(session)")
    attach(app, "4 Sent")
  }

  /// The notification prompt pairing raises sits in SpringBoard, outside the app.
  @MainActor
  private func allowNotifications(_ app: XCUIApplication) {
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let allow = springboard.buttons["Allow"]
    if allow.waitForExistence(timeout: 6) {
      allow.tap()
    } else {
      app.tap()
    }
  }

  @MainActor
  private func wait(for element: XCUIElement, labelBeginsWith prefix: String, timeout: TimeInterval) -> Bool {
    let predicate = NSPredicate(format: "label BEGINSWITH %@", prefix)
    return XCTWaiter().wait(for: [XCTNSPredicateExpectation(predicate: predicate, object: element)], timeout: timeout) == .completed
  }

  @MainActor
  private func attach(_ app: XCUIApplication, _ name: String) {
    let attachment = XCTAttachment(screenshot: app.screenshot())
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
  }
}

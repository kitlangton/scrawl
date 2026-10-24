import XCTest

/// A screenshot shared from Photos to Scrawl reaches the session, through the share extension.
/// Like `EndToEndTests`, it runs only through `ios/scripts/e2e.sh`, which also puts a screenshot
/// in the simulator's library.
final class ShareTests: XCTestCase {
  static let note = "The Action Button row has no icon"

  override func setUp() {
    continueAfterFailure = false
  }

  @MainActor
  func testAScreenshotSharedFromPhotosReachesTheSession() throws {
    let environment = ProcessInfo.processInfo.environment
    guard let link = environment["SCRAWL_E2E_LINK"], !link.isEmpty else {
      throw XCTSkip("Needs a Scrawl to send to: run ios/scripts/e2e.sh")
    }
    let session = environment["SCRAWL_E2E_SESSION"] ?? "E2E SESSION"
    let photos = XCUIApplication(bundleIdentifier: "com.apple.mobileslideshow")
    // Whatever an earlier run left open (a share sheet, the app switcher).
    photos.terminate()
    XCUIDevice.shared.press(.home)

    // Pair and pick the session in the app; the share sheet sends there too.
    let app = XCUIApplication()
    app.launchArguments = ["-scrawl-link", link]
    app.launch()
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    if springboard.buttons["Allow"].waitForExistence(timeout: 4) { springboard.buttons["Allow"].tap() }
    XCTAssert(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Connected'")).firstMatch.waitForExistence(timeout: 20))
    app.buttons["destination"].tap()
    let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", session)).firstMatch
    XCTAssert(row.waitForExistence(timeout: 15))
    row.tap()
    let sent = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "Sent to \(session)"))
    let sentBefore = sent.count

    photos.launch()
    for label in ["Continue", "Not Now", "OK"] where photos.buttons[label].waitForExistence(timeout: 2) {
      photos.buttons[label].tap()
    }
    // Photos reopens where it was left (a photo, a share sheet): go back to the library grid.
    let library = photos.buttons["LibraryTab"]
    if library.waitForExistence(timeout: 5) { library.tap() }
    let screenshot = photos.images.matching(NSPredicate(format: "label BEGINSWITH 'Screenshot'")).firstMatch
    XCTAssert(screenshot.waitForExistence(timeout: 10), "e2e.sh puts a screenshot in the library")
    screenshot.tap()
    let share = photos.buttons["Share"].firstMatch
    if !share.waitForExistence(timeout: 8) { screenshot.tap() }
    XCTAssert(share.waitForExistence(timeout: 8))
    share.tap()
    let scrawl = photos.cells.matching(identifier: "shareCell").matching(NSPredicate(format: "label == 'Scrawl'")).firstMatch
    XCTAssert(scrawl.waitForExistence(timeout: 10), "Scrawl should be in the share sheet")
    scrawl.tap()

    // The extension's sheet, hosted over Photos.
    let send = photos.buttons["Send"]
    XCTAssert(send.waitForExistence(timeout: 15), "Scrawl's share sheet should open")
    let field = photos.textViews.firstMatch.exists ? photos.textViews.firstMatch : photos.textFields.firstMatch
    XCTAssert(field.waitForExistence(timeout: 5))
    field.tap()
    field.typeText(Self.note)
    attach(photos, "1 Share sheet")
    XCTAssert(wait(until: send, isEnabled: true, timeout: 10))
    send.tap()
    XCTAssert(wait(until: send, exists: false, timeout: 30), "The share sheet should close once it's handed off")

    app.activate()
    let arrived = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in sent.count > sentBefore }, object: nil)
    XCTAssertEqual(XCTWaiter().wait(for: [arrived], timeout: 120), .completed, "The shared screenshot should reach \(session)")
    attach(app, "2 Sent")
  }

  @MainActor
  private func wait(until element: XCUIElement, isEnabled: Bool, timeout: TimeInterval) -> Bool {
    let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isEnabled == %@", NSNumber(value: isEnabled)), object: element)
    return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
  }

  @MainActor
  private func wait(until element: XCUIElement, exists: Bool, timeout: TimeInterval) -> Bool {
    let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == %@", NSNumber(value: exists)), object: element)
    return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
  }

  @MainActor
  private func attach(_ app: XCUIApplication, _ name: String) {
    let shot = XCTAttachment(screenshot: app.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}

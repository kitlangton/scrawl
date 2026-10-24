import SwiftUI
import UIKit

/// Share a screenshot or screen recording to Scrawl: it goes to the Mac like a recording made
/// in the app, with a note.
final class ShareViewController: UIViewController {
  private let model = ShareModel()

  override func viewDidLoad() {
    super.viewDidLoad()
    model.context = extensionContext
    let host = UIHostingController(rootView: ShareView(model: model))
    addChild(host)
    host.view.frame = view.bounds
    host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(host.view)
    host.didMove(toParent: self)
    Task { await model.load() }
  }
}

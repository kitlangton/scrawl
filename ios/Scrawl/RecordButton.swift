import ReplayKit
import SwiftUI

/// Opens iOS's broadcast sheet with Scrawl chosen. Only the system's picker can start a
/// broadcast, so this button taps it.
struct RecordButton: View {
  let isRecording: Bool
  @State private var picker = PickerHandle()

  var body: some View {
    Button {
      picker.open()
    } label: {
      Label(isRecording ? "Recording… stop from the red indicator" : "Record Screen", systemImage: isRecording ? "record.circle.fill" : "record.circle")
        .font(.headline)
        .frame(maxWidth: .infinity, minHeight: 34)
    }
    .buttonStyle(.borderedProminent)
    .controlSize(.large)
    .background(alignment: .center) {
      BroadcastPicker(handle: picker)
        .frame(width: 44, height: 44)
        .opacity(0.02)
        .accessibilityHidden(true)
    }
  }
}

@MainActor
final class PickerHandle {
  weak var view: RPSystemBroadcastPickerView?

  func open() {
    guard let button = view?.subviews.lazy.compactMap({ $0 as? UIButton }).first else { return }
    button.sendActions(for: .allTouchEvents)
  }
}

private struct BroadcastPicker: UIViewRepresentable {
  let handle: PickerHandle

  func makeUIView(context _: Context) -> RPSystemBroadcastPickerView {
    let picker = RPSystemBroadcastPickerView(frame: CGRect(x: 0, y: 0, width: 44, height: 44))
    picker.preferredExtension = Bundle.main.bundleIdentifier.map { "\($0).broadcast" }
    picker.showsMicrophoneButton = true
    handle.view = picker
    return picker
  }

  func updateUIView(_ picker: RPSystemBroadcastPickerView, context _: Context) {
    handle.view = picker
  }
}

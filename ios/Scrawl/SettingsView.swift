import SwiftUI

struct SettingsView: View {
  @Environment(AppModel.self) private var model
  @State private var confirmUnpair = false

  var body: some View {
    Form {
      Section {
        if let pairing = model.pairing {
          if let host = model.host {
            LabeledContent("Mac", value: host)
          }
          ForEach(pairing.urls, id: \.self) { url in
            LabeledContent(url.scheme == "https" ? "Endpoint" : "Address") {
              Text(url.absoluteString).font(.callout.monospaced()).textSelection(.enabled)
            }
          }
          Button {
            Task { await model.sendTest() }
          } label: {
            HStack {
              Label("Send a Test Capture", systemImage: "paperplane")
              if model.isSendingTest {
                Spacer()
                ProgressView()
              }
            }
          }
          .disabled(model.isSendingTest)
          .accessibilityIdentifier("send-test")
          Button("Unpair", systemImage: "xmark.circle", role: .destructive) { confirmUnpair = true }
        } else {
          Text("Not paired. Run scrawl link on your Mac and scan its code.")
            .foregroundStyle(.secondary)
        }
      } header: {
        Text("Pairing")
      } footer: {
        if model.pairing != nil {
          Text("A test capture is a card sent like a screenshot, to the session under Send to: it checks the whole way to OpenCode.")
        }
      }

      Section {
        Label("Flag a moment", systemImage: "flag.fill")
        Text("While recording, run Scrawl's Flag Moment from the Action button (Settings › Action Button › Shortcut) or Back Tap (Settings › Accessibility › Touch › Back Tap). Each flag gets its own keyframe, marked ⚑ for the agent.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
        Text("Saying “look here” or “now” as the screen changes works too: your Mac cuts a keyframe there.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
      } header: {
        Text("While recording")
      }

      Section {
        Text("Share screenshots or a screen recording from Photos (or the screenshot editor) to Scrawl, with a note. Screenshots each get a keyframe.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
      } header: {
        Text("Screenshots")
      }

      Section("About") {
        LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "–")
        Link(destination: URL(string: "https://github.com/kitlangton/scrawl#scrawl-for-iphone")!) {
          Label("Scrawl on GitHub", systemImage: "arrow.up.right.square")
        }
      }
    }
    .navigationTitle("Settings")
    .alert("Couldn't make the test capture", isPresented: Binding(get: { model.testProblem != nil }, set: { if !$0 { model.testProblem = nil } })) {
      Button("OK") {}
    } message: {
      Text(model.testProblem ?? "")
    }
    .confirmationDialog("Unpair this iPhone?", isPresented: $confirmUnpair, titleVisibility: .visible) {
      Button("Unpair", role: .destructive) { model.unpair() }
    } message: {
      Text("Recordings stay on the iPhone until you pair again.")
    }
  }
}

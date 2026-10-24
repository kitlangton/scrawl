import SwiftUI

struct HomeView: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    NavigationStack {
      List {
        if model.pairing == nil {
          PairSection()
        } else {
          MacSection()
        }
        RecordSection()
        CapturesSection()
      }
      .navigationTitle("Scrawl")
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          NavigationLink {
            SettingsView()
          } label: {
            Image(systemName: "gearshape")
          }
          .accessibilityLabel("Settings")
          .accessibilityIdentifier("settings")
        }
      }
      .refreshable { await model.refresh() }
      .navigationDestination(for: String.self) { id in
        CaptureDetailView(id: id)
      }
    }
  }
}

/// Whether the Mac answers, and what it's called.
private struct MacSection: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    Section {
      HStack(spacing: 12) {
        Image(systemName: "desktopcomputer")
          .font(.title2)
          .foregroundStyle(.secondary)
          .frame(width: 32)
        VStack(alignment: .leading, spacing: 2) {
          Text(model.host ?? "Your Mac").font(.headline)
          Text(status).font(.subheadline).foregroundStyle(statusColor)
            .accessibilityIdentifier("mac-status")
        }
        Spacer()
        if model.mac == .checking { ProgressView() }
      }
      .padding(.vertical, 2)
    } footer: {
      switch model.mac {
      case .unreachable:
        Text("Your iPhone and Mac need to be on the same Wi‑Fi or tailnet, with Scrawl (or scrawl serve) running on the Mac. Recordings wait on the iPhone until then.")
      case .unauthorized:
        Text("Run scrawl link on your Mac and scan the new code.")
      default:
        EmptyView()
      }
    }
  }

  private var status: String {
    switch model.mac {
    case .notPaired: "Not paired"
    case .checking: "Looking for your Mac…"
    case let .reachable(base): "Connected · \(base.host() ?? base.absoluteString)"
    case .unreachable: "Not reachable right now"
    case .unauthorized: "Doesn't know this iPhone anymore"
    }
  }

  private var statusColor: Color {
    switch model.mac {
    case .reachable: .green
    case .unreachable, .unauthorized: .orange
    default: .secondary
    }
  }
}

private struct RecordSection: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    Section {
      RecordButton(isRecording: model.recording != nil)
        .listRowInsets(EdgeInsets(top: 12, leading: 16, bottom: 12, trailing: 16))
      NavigationLink {
        DestinationView()
      } label: {
        LabeledContent("Send to") {
          Text(model.destination?.title ?? "Newest session")
            .lineLimit(1)
        }
      }
      .accessibilityIdentifier("destination")
    } header: {
      Text("Show an agent your phone")
    } footer: {
      Text("Records the whole screen, whatever app you open, with your voice: turn the microphone on in the sheet. Stop it from the red indicator and it goes to your Mac, which sends the keyframes and transcript to OpenCode. Control Center's Screen Recording works too: hold it and pick Scrawl.")
    }
  }
}

private struct CapturesSection: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    Section("Captures") {
      if model.captures.isEmpty {
        Text("Recordings, and screenshots you share to Scrawl, show up here with what became of them.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
          .padding(.vertical, 4)
      }
      ForEach(model.captures) { capture in
        NavigationLink(value: capture.id) {
          CaptureRow(capture: capture)
        }
        .swipeActions {
          Button("Delete", role: .destructive) { model.delete(capture) }
        }
      }
    }
  }
}

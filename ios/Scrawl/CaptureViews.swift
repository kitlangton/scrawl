import AVKit
import SwiftUI

struct CaptureRow: View {
  let capture: Capture
  @State private var thumbnail: UIImage?

  var body: some View {
    HStack(spacing: 12) {
      ZStack {
        RoundedRectangle(cornerRadius: 6, style: .continuous).fill(.quaternary)
        if let thumbnail {
          Image(uiImage: thumbnail).resizable().scaledToFill()
        } else {
          Image(systemName: capture.kind == .broadcast ? "record.circle" : "photo").foregroundStyle(.secondary)
        }
      }
      .frame(width: 36, height: 64)
      .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
      VStack(alignment: .leading, spacing: 3) {
        Text(capture.createdAt.formatted(.relative(presentation: .named)).capitalizingFirstLetter)
          .font(.subheadline.weight(.semibold))
        Text("\(capture.lengthText) · \(capture.statusTitle)")
          .font(.caption)
          .foregroundStyle(.secondary)
          .lineLimit(2)
      }
      Spacer(minLength: 0)
      StatusIcon(state: capture.state)
    }
    .task(id: capture.state) {
      thumbnail = await Thumbnail.image(for: capture)
    }
  }
}

struct StatusIcon: View {
  let state: Capture.State

  var body: some View {
    switch state {
    case .recording:
      Image(systemName: "record.circle.fill").foregroundStyle(.red).symbolEffect(.pulse)
    case .sending:
      ProgressView()
    case .accepted(.delivered):
      Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
    case .accepted:
      Image(systemName: "desktopcomputer").foregroundStyle(.secondary)
    case .waiting, .notPaired:
      Image(systemName: "clock").foregroundStyle(.orange)
    case .failed:
      Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
    }
  }
}

struct CaptureDetailView: View {
  let id: String
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var player: AVPlayer?
  @State private var confirmDelete = false
  private let store = CaptureStore.shared

  var body: some View {
    if let capture = model.captures.first(where: { $0.id == id }) {
      List {
        if let player {
          Section {
            VideoPlayer(player: player)
              .frame(height: 420)
              .listRowInsets(EdgeInsets())
          }
        }
        Section {
          HStack(alignment: .top, spacing: 12) {
            StatusIcon(state: capture.state)
            VStack(alignment: .leading, spacing: 4) {
              Text(capture.statusTitle).font(.headline)
              if let detail = capture.statusDetail {
                Text(detail).font(.subheadline).foregroundStyle(.secondary)
              }
              if capture.state == .waiting, let error = capture.lastError {
                Text(error).font(.caption).foregroundStyle(.secondary)
              }
            }
          }
          .padding(.vertical, 2)
          if capture.canRetry {
            Button("Send Now", systemImage: "arrow.up.circle") {
              Task { await model.retry(capture) }
            }
          }
        }
        Section {
          LabeledContent("Recorded", value: capture.createdAt.formatted(date: .abbreviated, time: .shortened))
          LabeledContent("Length", value: capture.lengthText)
          LabeledContent("Kind", value: capture.kind == .broadcast ? "Screen recording" : "Shared")
          LabeledContent("Send to", value: capture.destination?.title ?? "Newest session")
          if let note = capture.note {
            VStack(alignment: .leading, spacing: 4) {
              Text("Note").font(.caption).foregroundStyle(.secondary)
              Text(note)
            }
          }
        }
        if FileManager.default.fileExists(atPath: store.movie(id).path) {
          Section {
            ShareLink(items: [store.movie(id), store.timeline(id)].filter { FileManager.default.fileExists(atPath: $0.path) }) {
              Label("Share Recording", systemImage: "square.and.arrow.up")
            }
          } footer: {
            Text("The movie and its timeline. For a Mac that isn't paired: AirDrop both, then run scrawl import there.")
          }
        }
        Section {
          Button("Delete", systemImage: "trash", role: .destructive) { confirmDelete = true }
        }
      }
      .navigationTitle(capture.kind == .broadcast ? "Recording" : "Shared")
      .navigationBarTitleDisplayMode(.inline)
      .task(id: capture.state == .recording) {
        guard capture.state != .recording, player == nil, FileManager.default.fileExists(atPath: store.movie(id).path) else { return }
        player = AVPlayer(url: store.movie(id))
      }
      .confirmationDialog("Delete this capture from your iPhone?", isPresented: $confirmDelete, titleVisibility: .visible) {
        Button("Delete", role: .destructive) {
          model.delete(capture)
          dismiss()
        }
      } message: {
        Text("Anything already on your Mac stays there.")
      }
    } else {
      ContentUnavailableView("Deleted", systemImage: "trash")
    }
  }
}

private extension String {
  var capitalizingFirstLetter: String { prefix(1).uppercased() + dropFirst() }
}

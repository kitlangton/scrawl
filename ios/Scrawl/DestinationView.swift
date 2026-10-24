import SwiftUI

/// Which OpenCode session on the Mac captures go to.
struct DestinationView: View {
  @Environment(AppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var search = ""

  var body: some View {
    List {
      Section {
        row(title: "Newest session", detail: "The session that was active most recently on your Mac, when the capture arrives", selected: model.destination == nil) {
          choose(nil)
        }
      }
      Section {
        ForEach(sessions) { session in
          row(title: session.title, detail: detail(session), selected: model.destination?.session == session.id) {
            choose(Destination(session))
          }
        }
        if let destination = model.destination, !(model.sessionList?.sessions.contains { $0.id == destination.session } ?? false) {
          row(title: destination.title, detail: "Not among the sessions your Mac listed", selected: true) {}
        }
      } header: {
        if model.isLoadingSessions, model.sessionList == nil {
          HStack(spacing: 8) {
            ProgressView()
            Text("Asking your Mac…")
          }
        } else if let host = model.host {
          Text("Sessions on \(host)")
        }
      } footer: {
        if let error = model.sessionsError {
          Text(error)
        } else if model.sessionList?.routing == false {
          Text("Your Mac doesn't send captures to OpenCode (scrawl serve --no-opencode, or SCRAWL_NO_OPENCODE): they're saved there.")
        } else if model.pairing == nil {
          Text("Pair with your Mac to pick one of its sessions.")
        }
      }
    }
    .navigationTitle("Send to")
    .searchable(text: $search, prompt: "Sessions")
    .refreshable { await model.loadSessions() }
    .task { await model.loadSessions() }
  }

  private var sessions: [RemoteSession] {
    let all = model.sessionList?.sessions ?? []
    let query = search.trimmingCharacters(in: .whitespaces)
    guard !query.isEmpty else { return all }
    return all.filter { $0.title.localizedCaseInsensitiveContains(query) || ($0.directory?.localizedCaseInsensitiveContains(query) ?? false) }
  }

  private func detail(_ session: RemoteSession) -> String {
    [session.project, session.updatedAt.map { $0.formatted(.relative(presentation: .named)) }]
      .compactMap(\.self)
      .joined(separator: " · ")
  }

  private func choose(_ destination: Destination?) {
    model.destination = destination
    dismiss()
  }

  private func row(title: String, detail: String, selected: Bool, action: @escaping () -> Void) -> some View {
    Button(action: action) {
      HStack {
        VStack(alignment: .leading, spacing: 2) {
          Text(title).lineLimit(2)
          if !detail.isEmpty {
            Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
          }
        }
        Spacer()
        if selected {
          Image(systemName: "checkmark").foregroundStyle(Color.scrawlRed).fontWeight(.semibold)
        }
      }
    }
    // A list of choices, not of actions: rows read in the text colour.
    .tint(.primary)
  }
}

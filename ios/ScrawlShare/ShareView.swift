import SwiftUI

struct ShareView: View {
  @Bindable var model: ShareModel

  var body: some View {
    NavigationStack {
      Form {
        Section {
          preview
        } footer: {
          if model.skipped > 0 {
            Text("Scrawl sends the first \(ShareModel.maxScreenshots); \(model.skipped) more were left out.")
          }
        }
        Section {
          TextField("What should the agent look at?", text: $model.note, axis: .vertical)
            .lineLimit(2 ... 8)
        } footer: {
          Text("Goes with it, above the keyframes. Dictate it with the keyboard's microphone.")
        }
        Section {
          Picker("Send to", selection: $model.destination) {
            Text("Newest session").tag(Destination?.none)
            ForEach(model.choices, id: \.self) { destination in
              Text(destination.title).tag(Optional(destination))
            }
          }
        } footer: {
          if model.pairing == nil {
            Text("This iPhone isn't paired yet: it's kept on the phone until you pair Scrawl with your Mac.")
          } else if model.destination == nil {
            Text("Your Mac sends it to its most recently active OpenCode session.")
          }
        }
        if let error = model.error {
          Section {
            Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
          }
        }
      }
      .navigationTitle("Scrawl")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Cancel", role: .cancel) { model.cancel() }
        }
        ToolbarItem(placement: .confirmationAction) {
          if model.isSending {
            ProgressView()
          } else {
            Button("Send") { Task { await model.send() } }
              .disabled(!model.canSend)
          }
        }
      }
    }
  }

  @ViewBuilder private var preview: some View {
    switch model.content {
    case .loading:
      HStack {
        ProgressView()
        Text("Reading what you shared…").foregroundStyle(.secondary)
      }
    case let .screenshots(_, previews):
      ScrollView(.horizontal, showsIndicators: false) {
        HStack(spacing: 10) {
          ForEach(previews.indices, id: \.self) { index in
            Image(uiImage: previews[index])
              .resizable()
              .scaledToFit()
              .frame(height: 180)
              .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
              .overlay(alignment: .topLeading) {
                if previews.count > 1 { chip("\(index + 1)") }
              }
          }
        }
        .padding(.vertical, 4)
      }
    case let .movie(_, poster):
      HStack(spacing: 12) {
        if let poster {
          Image(uiImage: poster)
            .resizable()
            .scaledToFit()
            .frame(height: 120)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
        VStack(alignment: .leading, spacing: 4) {
          Text("Screen recording").font(.headline)
          Text("Your Mac transcribes what you said and picks the keyframes.")
            .font(.subheadline).foregroundStyle(.secondary)
        }
      }
    case .nothing:
      Label("Scrawl takes screenshots and screen recordings.", systemImage: "photo.on.rectangle.angled")
        .foregroundStyle(.secondary)
    }
  }

  /// The keyframe number, as the contact sheet shows it: a small white chip in the corner.
  private func chip(_ text: String) -> some View {
    Text(text)
      .font(.caption2.weight(.bold).monospacedDigit())
      .foregroundStyle(.black)
      .padding(.horizontal, 6)
      .padding(.vertical, 2)
      .background(.white, in: RoundedRectangle(cornerRadius: 4, style: .continuous))
      .padding(6)
  }
}

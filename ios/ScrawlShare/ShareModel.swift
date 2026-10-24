import Foundation
import ImageIO
import Observation
import UIKit
import UniformTypeIdentifiers

/// What was shared, and sending it: screenshots (up to `maxScreenshots`) or one screen recording.
@MainActor
@Observable
final class ShareModel {
  /// Screenshots are decoded one at a time while the movie is written, but the share sheet's
  /// memory is small; ten is plenty for one problem.
  static let maxScreenshots = 10

  enum Content {
    case loading
    /// Copies of the shared files, in this extension's temporary folder.
    case screenshots([URL], previews: [UIImage])
    case movie(URL, preview: UIImage?)
    case nothing

    var files: [URL] {
      switch self {
      case let .screenshots(files, _): files
      case let .movie(file, _): [file]
      case .loading, .nothing: []
      }
    }
  }

  var content = Content.loading
  var note = ""
  var destination: Destination? = DestinationStore.current
  var sessions: [RemoteSession] = DestinationStore.sessions?.sessions ?? []
  var isSending = false
  var error: String?
  let pairing = PairingStore.load()
  /// More screenshots were shared than are sent.
  var skipped = 0

  @ObservationIgnored weak var context: NSExtensionContext?

  var canSend: Bool {
    guard !isSending else { return false }
    switch content {
    case .screenshots, .movie: return true
    case .loading, .nothing: return false
    }
  }

  /// The sessions to pick from, with the current destination even when the Mac didn't list it.
  var choices: [Destination] {
    var list = sessions.map(Destination.init)
    if let destination, !list.contains(where: { $0.session == destination.session }) {
      list.insert(destination, at: 0)
    }
    return list
  }

  func load() async {
    let providers = (context?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
    if let movie = providers.first(where: { $0.hasItemConformingToTypeIdentifier(UTType.movie.identifier) }) {
      do {
        let url = try await Self.copyFile(from: movie, type: .movie)
        content = .movie(url, preview: await Self.poster(of: url))
      } catch {
        self.error = error.localizedDescription
        content = .nothing
      }
    } else {
      var files: [URL] = []
      var previews: [UIImage] = []
      let images = providers.filter { $0.hasItemConformingToTypeIdentifier(UTType.image.identifier) }
      skipped = max(0, images.count - Self.maxScreenshots)
      for provider in images.prefix(Self.maxScreenshots) {
        guard let file = try? await Self.copyFile(from: provider, type: .image),
              let source = CGImageSourceCreateWithURL(file as CFURL, nil),
              StillsMovie.pixelSize(source) != nil
        else { continue }
        files.append(file)
        if let preview = StillsMovie.image(from: source, maxPixels: 360) { previews.append(UIImage(cgImage: preview)) }
      }
      content = files.isEmpty ? .nothing : .screenshots(files, previews: previews)
    }
    await refreshSessions()
  }

  /// The Mac's sessions, fresh, when it answers quickly.
  func refreshSessions() async {
    guard let pairing, case let .reachable(base) = await MacClient.reach(pairing, timeout: 2),
          let list = try? await MacClient.sessions(at: base, token: pairing.token, timeout: 4)
    else { return }
    DestinationStore.sessions = list
    sessions = list.sessions
  }

  func send() async {
    isSending = true
    defer { isSending = false }
    let note = note.trimmingCharacters(in: .whitespacesAndNewlines)
    do {
      let capture: Capture
      switch content {
      case let .screenshots(files, _):
        capture = try await SharedCapture.screenshots(files, note: note.isEmpty ? nil : note, destination: destination)
      case let .movie(url, _):
        capture = try await SharedCapture.movie(url, note: note.isEmpty ? nil : note, destination: destination, hosted: pairing?.isHosted ?? false)
      case .loading, .nothing:
        return
      }
      removeFiles()
      DestinationStore.current = destination
      // The share sheet closes right after; the upload carries on without it.
      await Sender.send(capture.id, pairing: pairing, evenIfUnreachable: true)
      context?.completeRequest(returningItems: nil)
    } catch {
      self.error = error.localizedDescription
    }
  }

  func cancel() {
    removeFiles()
    context?.cancelRequest(withError: CocoaError(.userCancelled))
  }

  private func removeFiles() {
    for file in content.files { try? FileManager.default.removeItem(at: file) }
  }

  // MARK: - Loading

  /// Copies the shared file out before the provider deletes it.
  private static func copyFile(from provider: NSItemProvider, type: UTType) async throws -> URL {
    try await withCheckedThrowingContinuation { continuation in
      _ = provider.loadFileRepresentation(for: type, openInPlace: false) { url, _, error in
        guard let url else {
          continuation.resume(throwing: error ?? CocoaError(.fileReadUnknown))
          return
        }
        let ext = url.pathExtension.isEmpty ? (type.preferredFilenameExtension ?? "dat") : url.pathExtension
        let copy = FileManager.default.temporaryDirectory.appending(path: "\(UUID().uuidString).\(ext)")
        do {
          try FileManager.default.copyItem(at: url, to: copy)
          continuation.resume(returning: copy)
        } catch {
          continuation.resume(throwing: error)
        }
      }
    }
  }

  private static func poster(of url: URL) async -> UIImage? {
    await Thumbnail.image(of: url, maxPixels: 360)
  }
}

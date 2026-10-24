import Foundation

/// An OpenCode session on the Mac that captures go to. No destination lets the Mac choose: the
/// session the setup link named, else the most recently active one.
struct Destination: Codable, Equatable, Hashable, Sendable {
  var session: String
  var title: String
  var directory: String?

  init(session: String, title: String, directory: String? = nil) {
    self.session = session
    self.title = title.isEmpty ? session : title
    self.directory = directory
  }

  init(_ session: RemoteSession) {
    self.init(session: session.id, title: session.title, directory: session.directory)
  }

  /// The last folder of the session's project ("scrawl" for `/Users/kit/code/scrawl`).
  var project: String? { directory.map { URL(filePath: $0).lastPathComponent }.flatMap { $0.isEmpty ? nil : $0 } }
}

/// A session as the Mac lists it (`GET /sessions`).
struct RemoteSession: Codable, Equatable, Hashable, Identifiable, Sendable {
  var id: String
  var title: String
  var directory: String?
  /// Milliseconds since 1970.
  var updated: Double?

  var updatedAt: Date? { updated.map { Date(timeIntervalSince1970: $0 / 1000) } }
  var project: String? { Destination(self).project }
}

/// What the Mac said about its sessions, kept so the share sheet can offer them right away.
struct SessionList: Codable, Equatable, Sendable {
  /// The Mac's name ("Kit's Mac Studio").
  var host: String?
  /// Whether the Mac sends captures to OpenCode at all.
  var routing: Bool
  var sessions: [RemoteSession]
  var fetchedAt: Date
}

/// The chosen destination and the last session list, shared with the extensions.
enum DestinationStore {
  private static let destinationKey = "destination"
  private static let sessionsKey = "sessions"

  static var current: Destination? {
    get { decode(Destination.self, destinationKey) }
    set { encode(newValue, destinationKey) }
  }

  static var sessions: SessionList? {
    get { decode(SessionList.self, sessionsKey) }
    set { encode(newValue, sessionsKey) }
  }

  private static func decode<T: Decodable>(_ type: T.Type, _ key: String) -> T? {
    AppGroup.defaults.data(forKey: key).flatMap { try? JSONDecoder().decode(type, from: $0) }
  }

  private static func encode<T: Encodable>(_ value: T?, _ key: String) {
    if let value, let data = try? JSONEncoder().encode(value) {
      AppGroup.defaults.set(data, forKey: key)
    } else {
      AppGroup.defaults.removeObject(forKey: key)
    }
  }
}

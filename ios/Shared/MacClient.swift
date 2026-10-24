import Foundation

/// Talks to Scrawl's ingest server on the Mac (`Scrawl.app` or `scrawl serve`), or to an HTTPS
/// endpoint that speaks the same protocol.
enum MacClient {
  enum Reachability: Equatable, Sendable {
    /// The first address that answered `/health` with this token.
    case reachable(URL)
    /// An address answered, but not to this token: the pairing was replaced or revoked.
    case unauthorized
    case unreachable
  }

  /// Asks every address at once and takes the first that answers, as ScrawlKit does when a
  /// recording starts (LAN and Tailscale race; whichever this network can reach wins).
  static func reach(_ pairing: Pairing, timeout: TimeInterval = 3) async -> Reachability {
    await withTaskGroup(of: (URL, Int?).self) { group in
      for base in pairing.urls {
        group.addTask { (base, await status(get(base.appending(path: "health"), pairing.token, timeout))) }
      }
      var unauthorized = false
      for await (base, status) in group {
        if status == 200 {
          group.cancelAll()
          return .reachable(base)
        }
        if status == 401 || status == 403 { unauthorized = true }
      }
      return unauthorized ? .unauthorized : .unreachable
    }
  }

  enum SessionsError: Error, LocalizedError {
    /// The Mac's Scrawl is from before `GET /sessions`: it only answers `{"ok":true}`.
    case unsupported
    case unauthorized
    /// OpenCode on the Mac didn't answer.
    case mac(String)

    var errorDescription: String? {
      switch self {
      case .unsupported: "Update Scrawl on your Mac to pick a session from here."
      case .unauthorized: "Your Mac doesn't know this pairing anymore."
      case let .mac(message): message
      }
    }
  }

  /// The open root sessions on the Mac, newest first.
  static func sessions(at base: URL, token: String, timeout: TimeInterval = 8) async throws -> SessionList {
    let (data, response) = try await URLSession.shared.data(for: get(base.appending(path: "sessions"), token, timeout))
    let status = (response as? HTTPURLResponse)?.statusCode ?? 0
    if status == 401 || status == 403 { throw SessionsError.unauthorized }
    struct Reply: Decodable {
      var ok: Bool?
      var routing: Bool?
      var host: String?
      var sessions: [RemoteSession]?
      var error: String?
    }
    let reply = try? JSONDecoder().decode(Reply.self, from: data)
    guard (200 ..< 300).contains(status) else {
      throw SessionsError.mac(reply?.error ?? "Your Mac answered \(status).")
    }
    guard let reply, let sessions = reply.sessions else { throw SessionsError.unsupported }
    return SessionList(host: reply.host, routing: reply.routing ?? true, sessions: sessions, fetchedAt: Date())
  }

  static func get(_ url: URL, _ token: String, _ timeout: TimeInterval) -> URLRequest {
    var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: timeout)
    authorize(&request, token: token)
    return request
  }

  /// The bearer token, twice: iOS may drop or replace `Authorization` on background uploads, and
  /// the Mac also reads `X-Scrawl-Token`.
  static func authorize(_ request: inout URLRequest, token: String) {
    request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
    request.setValue(token, forHTTPHeaderField: "X-Scrawl-Token")
  }

  private static func status(_ request: URLRequest) async -> Int? {
    let response = try? await URLSession.shared.data(for: request).1
    return (response as? HTTPURLResponse)?.statusCode
  }
}

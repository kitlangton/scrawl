import Foundation
import Security

/// How this phone reaches Scrawl: the Mac's ingest addresses (Tailscale and LAN) or an HTTPS
/// endpoint, and the bearer token, from the setup link `scrawl link` prints:
/// `scrawl://scrawl?url=<base>[&url=<second>]&token=<token>[&session=ses_…]`.
struct Pairing: Codable, Equatable, Sendable {
  var urls: [URL]
  var token: String
  /// A session the link addresses every capture to (`scrawl link --session`).
  var session: String?

  init(urls: [URL], token: String, session: String? = nil) {
    self.urls = Self.baseURLs(urls.map(\.absoluteString))
    self.token = token.trimmingCharacters(in: .whitespacesAndNewlines)
    let session = session?.trimmingCharacters(in: .whitespacesAndNewlines)
    self.session = (session?.isEmpty == false) ? session : nil
  }

  var isValid: Bool { !urls.isEmpty && !token.isEmpty }

  /// An HTTPS endpoint takes 95 MB at most, so recordings for one are smaller and shorter.
  var isHosted: Bool { urls.contains { $0.scheme?.lowercased() == "https" } }

  enum Link: Equatable, Sendable {
    case pair(Pairing)
    case reset
  }

  /// Reads a setup link (or `…://scrawl?reset=1`). The scheme doesn't matter, so a link printed
  /// for a ScrawlKit app pairs this one too; the host must be `scrawl` (or the old `tether`) and
  /// only `http` and `https` addresses count.
  static func parse(_ url: URL) -> Link? {
    guard let scheme = url.scheme, !scheme.isEmpty,
          let host = url.host?.lowercased(), ["scrawl", "tether"].contains(host),
          let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems
    else { return nil }
    if items.contains(where: { $0.name == "reset" && ["1", "true"].contains($0.value?.lowercased() ?? "") }) {
      return .reset
    }
    let pairing = Pairing(
      urls: baseURLs(items.filter { $0.name == "url" }.compactMap(\.value)),
      token: items.first { $0.name == "token" }?.value ?? "",
      session: items.first { $0.name == "session" }?.value
    )
    return pairing.isValid ? .pair(pairing) : nil
  }

  /// A setup link inside pasted text (the terminal line, with spaces or quotes around it).
  static func parse(text: String) -> Link? {
    for word in text.split(whereSeparator: { $0.isWhitespace || "\"'<>".contains($0) }) {
      if let url = URL(string: String(word)), let link = parse(url) { return link }
    }
    return nil
  }

  /// `http(s)` base URLs without trailing slashes, each once, in order.
  static func baseURLs(_ candidates: [String]) -> [URL] {
    var seen = Set<String>()
    return candidates.compactMap { raw in
      var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
      while text.hasSuffix("/") { text.removeLast() }
      guard let url = URL(string: text), let scheme = url.scheme?.lowercased(),
            scheme == "http" || scheme == "https", url.host?.isEmpty == false,
            seen.insert(url.absoluteString).inserted
      else { return nil }
      return url
    }
  }
}

/// The pairing, in the Keychain group the app and its extensions share.
enum PairingStore {
  private static let service = "dev.kitlangton.scrawl.ios"
  private static let account = "pairing"

  /// `$(AppIdentifierPrefix)<app ID>`, from Info.plist (`ScrawlKeychainGroup`).
  private static let accessGroup: String? = {
    guard let group = Bundle.main.object(forInfoDictionaryKey: "ScrawlKeychainGroup") as? String,
          !group.isEmpty, !group.contains("$(")
    else { return nil }
    return group
  }()

  private static func query(group: String?) -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
    if let group { query[kSecAttrAccessGroup as String] = group }
    return query
  }

  static func load() -> Pairing? {
    for group in [accessGroup, nil] {
      var query = query(group: group)
      query[kSecReturnData as String] = true
      query[kSecMatchLimit as String] = kSecMatchLimitOne
      var result: CFTypeRef?
      if SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
         let data = result as? Data,
         let pairing = try? JSONDecoder().decode(Pairing.self, from: data)
      {
        let normalized = Pairing(urls: pairing.urls, token: pairing.token, session: pairing.session)
        return normalized.isValid ? normalized : nil
      }
      if accessGroup == nil { break }
    }
    return nil
  }

  @discardableResult
  static func save(_ pairing: Pairing) -> Bool {
    guard pairing.isValid, let data = try? JSONEncoder().encode(pairing) else { return false }
    clear()
    var item = query(group: accessGroup)
    item[kSecValueData as String] = data
    // Readable while locked after the first unlock: uploads finish in the background.
    item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    var status = SecItemAdd(item as CFDictionary, nil)
    if status == errSecMissingEntitlement, accessGroup != nil {
      // A build without the shared Keychain group: the app still pairs, the extensions can't see it.
      log.error("No Keychain group entitlement; pairing is kept for the app alone")
      item.removeValue(forKey: kSecAttrAccessGroup as String)
      status = SecItemAdd(item as CFDictionary, nil)
    }
    if status != errSecSuccess { log.error("Could not save the pairing (\(status))") }
    return status == errSecSuccess
  }

  static func clear() {
    for group in [accessGroup, nil] {
      SecItemDelete(query(group: group) as CFDictionary)
      if accessGroup == nil { break }
    }
  }
}

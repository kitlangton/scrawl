//! OS-owned installation transaction. No capture/config/feed access and no forced quits.
import AppKit
import Foundation
import Security
import Darwin

private let appID = "dev.simonvanlaak.apparent.preview"
private let appName = "Apparent Preview.app"
struct InstallError: LocalizedError {
  let message: String
  var errorDescription: String? { message }
}
private func require(_ condition: Bool, _ message: String) throws {
  if !condition { throw InstallError(message: message) }
}
struct AppInfo {
  let version: String
  let build: String
  let team: String?
}
func appInfo(_ url: URL, verify: (URL) throws -> String? = verifiedTeam) throws -> AppInfo {
  let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
  try require(attributes[.type] as? FileAttributeType == .typeDirectory, "An app path is a link or is not a folder. Move it aside in Finder first.")
  let plistURL = url.appendingPathComponent("Contents/Info.plist")
  let plistAttributes = try FileManager.default.attributesOfItem(atPath: plistURL.path)
  try require(plistAttributes[.type] as? FileAttributeType == .typeRegular, "The app metadata must be a regular file.")
  let object = try PropertyListSerialization.propertyList(from: Data(contentsOf: plistURL), format: nil)
  guard let p = object as? [String: Any], p["CFBundleIdentifier"] as? String == appID,
        p["CFBundleExecutable"] as? String == "apparent",
        let version = p["CFBundleShortVersionString"] as? String,
        let build = p["CFBundleVersion"] as? String else {
    throw InstallError(message: "This is not the expected Apparent Preview app. Nothing was replaced.")
  }
  try require(build.range(of: "^[0-9]+(\\.[0-9]+)*$", options: .regularExpression) != nil,
              "The app has an invalid build number.")
  return AppInfo(version: version, build: build, team: try verify(url))
}
func verifiedTeam(_ url: URL) throws -> String? {
  var code: SecStaticCode?
  guard SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(rawValue: 0), &code) == errSecSuccess, let code else {
    throw InstallError(message: "The app signature could not be read. Download the installer again.")
  }
  let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode)
  try require(SecStaticCodeCheckValidity(code, flags, nil) == errSecSuccess,
              "The app signature is invalid. Nothing was installed.")
  var information: CFDictionary?
  try require(SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
              "The app signing identity could not be read.")
  return (information as? [String: Any])?[kSecCodeInfoTeamIdentifier as String] as? String
}
private func identity(_ url: URL) throws -> String {
  let a = try FileManager.default.attributesOfItem(atPath: url.path)
  try require(a[.type] as? FileAttributeType == .typeDirectory, "An installation folder became a link. Nothing was replaced.")
  return "\(a[.systemNumber]!)·\(a[.systemFileNumber]!)"
}
func runningApp(_ target: URL) -> Bool {
  let prefix = target.standardizedFileURL.path + "/"
  return NSWorkspace.shared.runningApplications.contains { app in
    app.bundleIdentifier == appID || app.executableURL?.standardizedFileURL.path.hasPrefix(prefix) == true
  }
}
final class InstallTransaction {
  let source: URL
  let applications: URL
  let verify: (URL) throws -> String?
  let running: (URL) -> Bool
  init(source: URL, applications: URL = URL(fileURLWithPath: "/Applications"),
       verify: @escaping (URL) throws -> String? = verifiedTeam,
       running: @escaping (URL) -> Bool = runningApp) {
    self.source = source.standardizedFileURL; self.applications = applications.standardizedFileURL
    self.verify = verify; self.running = running
  }
  var target: URL { applications.appendingPathComponent(appName) }
  func inspect() throws -> [String: Any] {
    let info = try appInfo(source, verify: verify)
    try require(source.resolvingSymlinksInPath() == source, "The installer payload path must not contain links.")
    try require(!source.path.hasPrefix(target.path + "/") && source != target,
                "Run this installer from the downloaded disk image, not inside the installed app.")
    let folder = try identity(applications)
    try require(applications.resolvingSymlinksInPath() == applications, "Applications must not be a symbolic link.")
    _ = folder
    let exists = FileManager.default.fileExists(atPath: target.path) || (try? target.resourceValues(forKeys: [.isSymbolicLinkKey]).isSymbolicLink) == true
    if exists {
      let old = try appInfo(target, verify: verify)
      try require(old.team == info.team, "The existing app has a different signing identity. Move it aside in Finder before installing.")
      try require(old.build.compare(info.build, options: .numeric) != .orderedDescending,
                  "A newer app is already installed. Keep it, or move it aside in Finder first.")
    }
    try require(!running(target), "Finish any recording, then quit Apparent Preview yourself and try again. The installer will not quit it.")
    return ["version": info.version, "build": info.build, "needsReplacement": exists, "destination": target.path]
  }
  func install(replace: Bool) throws -> [String: Any] {
    let state = try inspect()
    try require(state["needsReplacement"] as? Bool != true || replace,
                "Confirm replacement of the existing app before installing.")
    let fm = FileManager.default
    let directoryIdentity = try identity(applications)
    let oldIdentity: String?
    if state["needsReplacement"] as? Bool == true { oldIdentity = try identity(target) }
    else { oldIdentity = nil }
    let stage = applications.appendingPathComponent(".apparent-install-\(UUID().uuidString).app")
    let backup = applications.appendingPathComponent(".apparent-previous-\(UUID().uuidString).backup")
    var stageIdentity: String?
    defer {
      if let staged = stageIdentity, (try? identity(applications)) == directoryIdentity,
         (try? identity(stage)) == staged { try? fm.removeItem(at: stage) }
    }
    try fm.copyItem(at: source, to: stage)
    stageIdentity = try identity(stage)
    let new = try appInfo(stage, verify: verify)
    let original = try appInfo(source, verify: verify)
    try require(new.build == original.build && new.team == original.team, "The copied app did not verify. The old app is untouched.")
    _ = try inspect() // Recheck running app, version and identity after the potentially slow copy.
    try require(try identity(applications) == directoryIdentity, "Applications changed during installation. Try again.")
    if let old = oldIdentity {
      try require(try identity(target) == old, "The installed app changed while copying. Try again.")
    } else {
      try require(!fm.fileExists(atPath: target.path), "Another app appeared at the destination. Try again.")
    }
    // Rename relative to an open, no-follow directory; never traverse a replaced Applications link.
    let fd = open(applications.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
    try require(fd >= 0, "Applications could not be opened safely.")
    defer { close(fd) }
    var statbuf = stat()
    try require(fstat(fd, &statbuf) == 0 && "\(statbuf.st_dev)·\(statbuf.st_ino)" == directoryIdentity,
                "Applications changed during installation. Try again.")
    try require(!running(target), "Apparent Preview started while copying. Quit it yourself before installing.")
    var backedUp = false
    if let old = oldIdentity {
      try require(renameat(fd, appName, fd, backup.lastPathComponent) == 0, "The old app could not be preserved. Nothing was replaced.")
      backedUp = true
      try require((try? identity(backup)) == old, "The destination changed during replacement. The preserved item was left beside the app; nothing was deleted.")
    }
    do {
      try require(!running(target), "Apparent Preview started during replacement. Quit it yourself before installing.")
      // RENAME_EXCL prevents replacing a newly appeared file, app or symlink.
      try require(renameatx_np(fd, stage.lastPathComponent, fd, appName, UInt32(RENAME_EXCL)) == 0,
                  "The app could not be installed. Check Applications in Finder.")
      _ = try appInfo(target, verify: verify)
    } catch {
      // Restore only into an absent destination, never follow or overwrite a raced-in link.
      if let staged = stageIdentity, (try? identity(target)) == staged {
        _ = renameatx_np(fd, appName, fd, stage.lastPathComponent, UInt32(RENAME_EXCL))
      }
      if backedUp { _ = renameatx_np(fd, backup.lastPathComponent, fd, appName, UInt32(RENAME_EXCL)) }
      throw error
    }
    var result: [String: Any] = ["installed": true, "destination": target.path]
    // Preserve the old signed app, rather than deleting a previous installation.
    if backedUp { result["previousApp"] = backup.path }
    return result
  }
}

#if !INSTALLER_TEST
private func reply(_ object: [String: Any]) {
  if let data = try? JSONSerialization.data(withJSONObject: object) { FileHandle.standardOutput.write(data) }
}
private func run() throws {
  let installer = Bundle.main.bundleURL
  let payload = installer.appendingPathComponent("Contents/SharedSupport/" + appName)
  let installerTeam = try verifiedTeam(installer)
  let payloadInfo = try appInfo(payload)
  try require(installerTeam == payloadInfo.team, "The payload signing identity does not match this installer. Download it again.")
  try require(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String == payloadInfo.version
              && Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String == payloadInfo.build,
              "The payload version does not match this installer. Download it again.")
  let transaction = InstallTransaction(source: payload)
  if CommandLine.arguments.count == 3 && CommandLine.arguments[1] == "--request" {
    guard let data = CommandLine.arguments[2].data(using: .utf8),
          let request = try JSONSerialization.jsonObject(with: data) as? [String: Any],
          let action = request["action"] as? String else { throw InstallError(message: "Invalid installer request.") }
    switch action {
    case "status": reply(try transaction.inspect())
    case "install": reply(try transaction.install(replace: request["replace"] as? Bool == true))
    case "open":
      _ = try appInfo(transaction.target)
      var done = false
      var error: Error?
      NSWorkspace.shared.openApplication(at: transaction.target, configuration: NSWorkspace.OpenConfiguration()) { app, failure in
        error = failure
        if app?.bundleIdentifier != appID && failure == nil { error = InstallError(message: "The installed app did not open.") }
        done = true
      }
      let deadline = Date().addingTimeInterval(15)
      while !done && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
      try require(done, "Opening the app timed out. Open it from Applications.")
      if let error { throw error }; reply(["opened": true])
    default: throw InstallError(message: "Unknown installer action.")
    }
    return
  }
  try require(CommandLine.arguments.count == 1, "Unexpected installer arguments.")
  let runtime = payload.appendingPathComponent("Contents/Resources/settings/runtime/Electron.app/Contents/MacOS/Electron")
  let ui = installer.appendingPathComponent("Contents/Resources/installer-ui")
  let args = [runtime.path, ui.path, "--installer-adapter=" + installer.appendingPathComponent("Contents/MacOS/Installer").path]
  var cargs = args.map { strdup($0) } + [nil]
  defer { cargs.forEach { free($0) } }
  _ = cargs.withUnsafeMutableBufferPointer { execv(runtime.path, $0.baseAddress!) }
  throw InstallError(message: "The installer could not open. Drag Apparent Preview to Applications in Finder.")
}
@main struct InstallerMain {
  static func main() {
    do { try run() } catch {
      if CommandLine.arguments.contains("--request") { reply(["error": error.localizedDescription]); exit(1) }
      let app = NSApplication.shared; app.setActivationPolicy(.regular); app.activate(ignoringOtherApps: true)
      let alert = NSAlert(); alert.messageText = "Installer could not open"; alert.informativeText = error.localizedDescription; alert.runModal(); exit(1)
    }
  }
}
#endif

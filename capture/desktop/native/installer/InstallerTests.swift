// Compile with -D INSTALLER_TEST alongside Installer.swift. Uses only owned temp fixtures.
import Foundation

@main struct InstallerTests {
  static let fm = FileManager.default
  static func app(_ url: URL, build: String = "12.1", id: String = "dev.simonvanlaak.apparent.preview") throws {
    try fm.createDirectory(at: url.appendingPathComponent("Contents/MacOS"), withIntermediateDirectories: true)
    let p = ["CFBundleIdentifier": id, "CFBundleExecutable": "apparent", "CFBundleShortVersionString": "0.12.1", "CFBundleVersion": build]
    try PropertyListSerialization.data(fromPropertyList: p, format: .xml, options: 0).write(to: url.appendingPathComponent("Contents/Info.plist"))
    try Data(build.utf8).write(to: url.appendingPathComponent("Contents/MacOS/apparent"))
  }
  static func check(_ value: Bool, file: StaticString = #filePath, line: UInt = #line) {
    precondition(value, file: file, line: line)
  }
  static func rejects(expected: String? = nil, file: StaticString = #filePath, line: UInt = #line, _ operation: () throws -> Void) {
    do {try operation(); fatalError("Unsafe installation was accepted", file: file, line: line)} catch {
      if let expected {check(error.localizedDescription.contains(expected), file: file, line: line)}
    }
  }
  static func main() throws {
    let root = fm.temporaryDirectory.appendingPathComponent("apparent-installer-\(UUID().uuidString)").resolvingSymlinksInPath()
    try fm.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? fm.removeItem(at: root) }
    var count = 0
    func scenario(_ body: (URL, URL) throws -> Void) throws {
      count += 1
      FileHandle.standardError.write(Data("Installer safe-copy fixture \(count)\n".utf8))
      let base = root.appendingPathComponent("case-\(count)")
      let source = base.appendingPathComponent("Payload.app"), applications = base.appendingPathComponent("Applications")
      try fm.createDirectory(at: applications, withIntermediateDirectories: true)
      try app(source)
      try body(source, applications)
    }
    let signature: (URL) throws -> String? = { _ in "fixture-team" }
    try scenario { source, folder in
      let installer = InstallTransaction(source: source, applications: folder, verify: signature, running: {_ in false})
      let result = try installer.install(replace: false)
      check(result["installed"] as? Bool == true)
      check(try Data(contentsOf: installer.target.appendingPathComponent("Contents/MacOS/apparent")) == Data("12.1".utf8))
      check(fm.fileExists(atPath: source.path))
    }
    try scenario { source, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app"); try app(target, build: "11.1")
      let installer = InstallTransaction(source: source, applications: folder, verify: signature, running: {_ in false})
      rejects { _ = try installer.install(replace: false) }
      let result = try installer.install(replace: true)
      let backup = URL(fileURLWithPath: result["previousApp"] as! String)
      check(backup.pathExtension == "backup")
      check(try Data(contentsOf: backup.appendingPathComponent("Contents/MacOS/apparent")) == Data("11.1".utf8))
      check(try appInfo(backup, verify: signature).build == "11.1")
      check(try appInfo(target, verify: signature).build == "12.1")
    }
    try scenario { source, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app"); try app(target, build: "13.1")
      rejects { _ = try InstallTransaction(source: source, applications: folder, verify: signature, running: {_ in false}).install(replace: true) }
      check(try appInfo(target, verify: signature).build == "13.1")
    }
    try scenario { source, folder in
      rejects { _ = try InstallTransaction(source: source, applications: folder, verify: signature, running: {_ in true}).install(replace: true) }
      check(try fm.contentsOfDirectory(atPath: folder.path).isEmpty)
    }
    try scenario { source, folder in
      let other = folder.appendingPathComponent("Apparent Preview.app"); try app(other, id: "other.app")
      rejects { _ = try InstallTransaction(source: source, applications: folder, verify: signature, running: {_ in false}).install(replace: true) }
      check(fm.fileExists(atPath: other.path))
    }
    try scenario { source, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app")
      try fm.createSymbolicLink(at: target, withDestinationURL: source)
      rejects { _ = try InstallTransaction(source: source, applications: folder, verify: signature, running: {_ in false}).install(replace: true) }
      check(try fm.destinationOfSymbolicLink(atPath: target.path) == source.path)
    }
    try scenario { source, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app"); try app(target, build: "11.1")
      let verifier: (URL) throws -> String? = { url in
        if url.lastPathComponent.hasPrefix(".apparent-install-") {throw InstallError(message: "fixture copied signature failure")}
        return "fixture-team"
      }
      rejects { _ = try InstallTransaction(source: source, applications: folder, verify: verifier, running: {_ in false}).install(replace: true) }
      check(try appInfo(target, verify: signature).build == "11.1")
      check(try fm.contentsOfDirectory(atPath: folder.path) == ["Apparent Preview.app"])
    }
    try scenario { source, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app"); try app(target, build: "11.1")
      var polls = 0
      let running: (URL) -> Bool = { _ in
        polls += 1
        if polls == 4 { try! fm.createSymbolicLink(at: target, withDestinationURL: source) }
        return false
      }
      rejects { _ = try InstallTransaction(source: source, applications: folder, verify: signature, running: running).install(replace: true) }
      check(try fm.destinationOfSymbolicLink(atPath: target.path) == source.path)
      let backups = try fm.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil).filter {$0.lastPathComponent.hasPrefix(".apparent-previous-")}
      check(backups.count == 1)
      check(try appInfo(backups[0], verify: signature).build == "11.1")
    }
    try scenario { source, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app"); try app(target, build: "11.1")
      var checkedExistingTeam = false
      let differentTeam: (URL) throws -> String? = { url in
        // URL directory hints can change once the fixture app exists. Match the path.
        if url.standardizedFileURL.path == target.standardizedFileURL.path {
          checkedExistingTeam = true
          return "other-team"
        }
        return "fixture-team"
      }
      rejects(expected: "different signing identity") { _ = try InstallTransaction(source: source, applications: folder, verify: differentTeam, running: {_ in false}).install(replace: true) }
      check(checkedExistingTeam)
      check(try appInfo(target, verify: signature).build == "11.1")
    }
    try scenario { _, folder in
      let target = folder.appendingPathComponent("Apparent Preview.app"); try app(target, build: "11.1")
      let nested = target.appendingPathComponent("Nested.app"); try app(nested)
      rejects { _ = try InstallTransaction(source: nested, applications: folder, verify: signature, running: {_ in false}).install(replace: true) }
      check(fm.fileExists(atPath: nested.path))
    }
    print("Installer safe-copy fixtures passed: \(count); no real app or permissions changed")
  }
}

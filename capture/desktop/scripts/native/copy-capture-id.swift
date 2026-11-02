// Disposable GitHub-hosted macOS only. No recording, TCC edits or real captures.
// Compile with swiftc; invoke with installed .app and a fresh evidence directory.
import Foundation
import AppKit
import Vision
import CoreGraphics
import ApplicationServices
import Darwin

struct Failure: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}
func require(_ condition: @autoclosure () throws -> Bool, _ message: String) throws {
    if try !condition() { throw Failure(message) }
}
let fm = FileManager.default
let inherited = ProcessInfo.processInfo.environment
// Refuse before creating files, reading clipboard, launching apps or capturing pixels.
guard inherited["CI"] == "true", inherited["GITHUB_ACTIONS"] == "true",
      inherited["RUNNER_ENVIRONMENT"] == "github-hosted", inherited["RUNNER_OS"] == "macOS",
      inherited["APPARENT_COPY_ID_NATIVE_CI"] == "1", CommandLine.arguments.count == 3 else {
    fputs("Refusing non-GitHub-hosted native fixture invocation\n", stderr); exit(64)
}
let app = URL(fileURLWithPath: CommandLine.arguments[1]).standardizedFileURL
let output = URL(fileURLWithPath: CommandLine.arguments[2]).standardizedFileURL
let runner = URL(fileURLWithPath: inherited["RUNNER_TEMP"] ?? "/invalid").resolvingSymlinksInPath()
let deadline = Date().addingTimeInterval(150)
var env = inherited
var owned: Process?
var logs: [FileHandle] = []
var report: [String: Any] = ["passed": false, "nativeProof": "UNRUN until real macos-26 execution",
    "scope": "Installed Rust GPUI; generated fixtures and owned-window screenshots only",
    "checks": [[String: Any]]()]
var fixture: URL?
var sequence = 0
func remaining(_ cap: TimeInterval) throws -> TimeInterval {
    let value = min(cap, deadline.timeIntervalSinceNow)
    try require(value > 0, "Overall native fixture deadline exceeded")
    return value
}
func run(_ path: String, _ args: [String], cap: TimeInterval = 5) throws -> String {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: path)
    process.arguments = args
    process.environment = env
    // File-backed output avoids pipe-capacity deadlocks on demo diagnostics.
    let file = output.appendingPathComponent("subprocess-\(sequence).log")
    sequence += 1
    fm.createFile(atPath: file.path, contents: nil, attributes: [.posixPermissions: 0o600])
    let handle = try FileHandle(forWritingTo: file)
    defer { try? handle.close() }
    process.standardOutput = handle; process.standardError = handle
    let until = Date().addingTimeInterval(try remaining(cap))
    try process.run()
    while process.isRunning && Date() < until { Thread.sleep(forTimeInterval: 0.05) }
    if process.isRunning {
        process.terminate(); Thread.sleep(forTimeInterval: 0.2)
        if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        throw Failure("Subprocess timeout: \(path)")
    }
    let text = String(data: try Data(contentsOf: file), encoding: .utf8) ?? ""
    try require(process.terminationStatus == 0, "\(path) failed: \(text.prefix(2000))")
    return text.trimmingCharacters(in: .whitespacesAndNewlines)
}
func stop() throws {
    guard let child = owned else { return }
    if child.isRunning {
        child.terminate()
        let until = Date().addingTimeInterval(2)
        while child.isRunning && Date() < until { Thread.sleep(forTimeInterval: 0.05) }
        if child.isRunning { kill(child.processIdentifier, SIGKILL) }
        let killedUntil = Date().addingTimeInterval(1)
        while child.isRunning && Date() < killedUntil { Thread.sleep(forTimeInterval: 0.05) }
    }
    try require(!child.isRunning, "Owned preview did not stop")
    owned = nil
}
struct OwnedWindow {
    let id: CGWindowID
    let bounds: CGRect
}
func window() throws -> OwnedWindow {
    guard let child = owned, child.isRunning else { throw Failure("Owned preview exited") }
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    let matches = rows.filter {
        ($0[kCGWindowOwnerPID as String] as? Int32) == child.processIdentifier &&
        ($0[kCGWindowLayer as String] as? Int) == 0 &&
        (($0[kCGWindowBounds as String] as? [String: Any])?["Width"] as? Double ?? 0) > 400
    }
    try require(matches.count == 1, "Expected exactly one owned GPUI review window; got \(matches.count)")
    let row = matches[0]
    guard let id = row[kCGWindowNumber as String] as? UInt32,
          let dict = row[kCGWindowBounds as String] as? [String: Any],
          let bounds = CGRect(dictionaryRepresentation: dict as CFDictionary) else { throw Failure("Invalid owned bounds") }
    return OwnedWindow(id: id, bounds: bounds)
}
func postOwnedWindowClick(_ expected: OwnedWindow, at point: CGPoint) throws {
    guard let child = owned, child.isRunning else { throw Failure("Owned preview exited before input") }
    let current = try window()
    try require(current.id == expected.id && current.bounds == expected.bounds && current.bounds.contains(point),
                "Input target changed or escaped the owned window")
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    let hit = rows.first { row in
        guard (row[kCGWindowAlpha as String] as? Double ?? 1) > 0,
              let dict = row[kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: dict as CFDictionary) else { return false }
        return rect.contains(point)
    }
    // CGWindowList is a visual stacking list, not an input hit-test: transparent
    // or mouse-ignoring system surfaces can cover a window's rectangle. Ask AX
    // for the actual element at the observed point and keep the exact PID gate.
    let system = AXUIElementCreateSystemWide()
    _ = AXUIElementSetMessagingTimeout(system, 1)
    var element: AXUIElement?
    let axError = AXUIElementCopyElementAtPosition(system, Float(point.x), Float(point.y), &element)
    var hitPID: pid_t = -1
    if let element, axError == .success { _ = AXUIElementGetPid(element, &hitPID) }
    let inputAllowed = CGPreflightPostEventAccess()
    var inputs = report["inputTargets"] as? [[String: Any]] ?? []
    inputs.append(["expectedPID": child.processIdentifier, "hitPID": hitPID,
        "axError": axError.rawValue, "postEventAccess": inputAllowed,
        "visualRectanglePID": hit?[kCGWindowOwnerPID as String] as? Int32 ?? -1,
        "visualRectangleLayer": hit?[kCGWindowLayer as String] as? Int ?? -1])
    report["inputTargets"] = inputs
    try require(axError == .success && hitPID == child.processIdentifier,
                "The observed input point does not hit the exact owned app")
    try require(inputAllowed, "The fixture has no native event-posting permission; no input sent")
    for type in [CGEventType.mouseMoved, .leftMouseDown, .leftMouseUp] {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else {
            throw Failure("Could not create owned-window mouse event")
        }
        event.post(tap: .cghidEventTap)
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        if type == .mouseMoved {
            guard let position = CGEvent(source: nil)?.location else { throw Failure("Could not read pointer position") }
            try require(abs(position.x - point.x) <= 2 && abs(position.y - point.y) <= 2,
                        "Pointer did not settle at the observed target")
        }
    }
}
func focusAndSize(_ width: Int? = nil) throws {
    guard let child = owned else { throw Failure("No owned app") }
    guard let running = NSRunningApplication(processIdentifier: child.processIdentifier) else {
        throw Failure("Owned app is absent from the running-application registry")
    }
    let accepted = running.activate(options: [.activateIgnoringOtherApps])
    let size = width.map { "set size of window 1 to {\($0), 700}\nset position of window 1 to {40, 40}" } ?? ""
    let axFrontmost = try run("/usr/bin/osascript", ["-e", """
    tell application "System Events"
        set p to first application process whose unix id is \(child.processIdentifier)
        tell p
            if (count of windows) is not 1 then error "Expected one owned AX window"
            set frontmost to true
            \(size)
            perform action "AXRaise" of window 1
            return frontmost
        end tell
    end tell
    """])
    // Refresh AppKit's run-loop-cached activation properties before deciding
    // whether a normal observed-window activation click is necessary.
    RunLoop.main.run(until: Date().addingTimeInterval(0.15))
    var attempts = report["focus"] as? [[String: Any]] ?? []
    attempts.append(["pid": child.processIdentifier, "activationAccepted": accepted,
        "axFrontmost": axFrontmost, "isActive": running.isActive,
        "frontmostPID": NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1])
    report["focus"] = attempts
    if !running.isActive || NSWorkspace.shared.frontmostApplication?.processIdentifier != child.processIdentifier {
        let (before, boxes) = try screenshot("before-activation")
        let box = try locate("Capture review", in: boxes)
        let point = CGPoint(x: before.bounds.minX + box.midX * before.bounds.width,
                            y: before.bounds.minY + (1 - box.midY) * before.bounds.height)
        // Only the non-interactive header is used for activation; the exact-PID
        // foreground gate below still precedes every actual button click.
        try postOwnedWindowClick(before, at: point)
    }
    let until = Date().addingTimeInterval(try remaining(3))
    var frontmost: pid_t?
    repeat {
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
        if running.isActive && frontmost == child.processIdentifier { break }
    } while Date() < until && child.isRunning
    attempts.append(["pid": child.processIdentifier, "activationAccepted": accepted,
        "isActive": running.isActive, "frontmostPID": frontmost ?? -1])
    report["focus"] = attempts
    try require(running.isActive && frontmost == child.processIdentifier,
                "Focus is not the exact owned app PID after bounded activation")
}
struct TextBox { let text: String; let value: VNRecognizedText; let bounds: CGRect }
func screenshot(_ label: String) throws -> (OwnedWindow, [TextBox]) {
    let w = try window()
    let name = "\(sequence)-\(label).png"
    let url = output.appendingPathComponent(name)
    _ = try run("/usr/sbin/screencapture", ["-x", "-o", "-l", String(w.id), url.path], cap: 3)
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = false
    try VNImageRequestHandler(url: url).perform([request])
    let boxes = (request.results ?? []).compactMap { item -> TextBox? in
        guard let candidate = item.topCandidates(1).first else { return nil }
        return TextBox(text: candidate.string, value: candidate, bounds: item.boundingBox)
    }
    var shots = report["screenshots"] as? [[String: Any]] ?? []
    shots.append(["file": name, "pid": owned!.processIdentifier, "windowID": w.id,
        "bounds": [w.bounds.minX, w.bounds.minY, w.bounds.width, w.bounds.height],
        "ocr": boxes.map { ["text": $0.text, "box": [$0.bounds.minX, $0.bounds.minY, $0.bounds.width, $0.bounds.height]] }])
    report["screenshots"] = shots
    return (w, boxes)
}
// GPUI text need not be AX-labelled. Use Vision substring boxes on the header only,
// never guessed full-screen coordinates, synthetic content or another app's AX tree.
func locate(_ label: String, in boxes: [TextBox]) throws -> CGRect {
    let matches = try boxes.filter { $0.bounds.midY > 0.86 }.compactMap { item -> CGRect? in
        guard let range = item.text.range(of: label) else { return nil }
        if label == "Copy" && item.text != "Copy" { return nil }
        return try item.value.boundingBox(for: range)?.boundingBox
    }
    try require(matches.count == 1, "Missing/ambiguous header label \(label): \(matches.count)")
    return matches[0]
}
func click(_ label: String) throws {
    try focusAndSize()
    let (before, boxes) = try screenshot("before-click")
    let box = try locate(label, in: boxes)
    let current = try window()
    try require(current.id == before.id && current.bounds == before.bounds, "Window moved during OCR")
    RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    try require(NSWorkspace.shared.frontmostApplication?.processIdentifier == owned?.processIdentifier, "Focus changed before click")
    let point = CGPoint(x: current.bounds.minX + box.midX * current.bounds.width,
                        y: current.bounds.minY + (1 - box.midY) * current.bounds.height)
    try postOwnedWindowClick(current, at: point)
    Thread.sleep(forTimeInterval: 0.25)
}
func copyCheck(_ label: String, expected: String, caseName: String) throws {
    let pb = NSPasteboard.general
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1, pixelsHigh: 1,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 4, bitsPerPixel: 32)!
    bitmap.setColor(.magenta, atX: 0, y: 0)
    let png = bitmap.representation(using: .png, properties: [:])!
    pb.clearContents()
    try require(pb.setString("sentinel-\(caseName)", forType: .string), "Seed text failed")
    try require(pb.setData(png, forType: .png), "Seed PNG failed")
    let seeded = pb.changeCount
    try require(pb.types?.contains(.png) == true, "PNG seed missing")
    try click(label)
    let until = Date().addingTimeInterval(try remaining(4))
    while pb.changeCount == seeded && Date() < until { Thread.sleep(forTimeInterval: 0.05) }
    let text = pb.string(forType: .string)
    let types = (pb.types ?? []).map { $0.rawValue }
    var checks = report["checks"] as! [[String: Any]]
    checks.append(["case": caseName, "expected": expected, "actual": text ?? "<nil>",
        "seedChangeCount": seeded, "changeCount": pb.changeCount, "types": types])
    report["checks"] = checks
    try require(pb.changeCount > seeded, "\(caseName): no fresh clipboard dispatch")
    try require(text == expected, "\(caseName): clipboard is not exact selected basename")
    // Strict string-only allowlist rejects PNG/TIFF, file URLs, paths, rich text,
    // custom summary types and any stale seeded item, not merely known image UTIs.
    let allowed: Set<String> = [NSPasteboard.PasteboardType.string.rawValue,
        "NSStringPboardType", "public.utf16-plain-text", "public.utf16-external-plain-text"]
    try require(!types.isEmpty && types.allSatisfy { allowed.contains($0) }, "Non-text clipboard format survived")
    try require(pb.pasteboardItems?.count == 1, "Unexpected extra clipboard items")
    _ = try screenshot(caseName)
}
func launch(_ executable: URL, disabled: Bool) throws {
    let config = fixture!.appendingPathComponent("config.json")
    let data: [String: Any] = disabled ? ["onboarding": ["seen": true], "experiments": ["copyCaptureId": false]] : ["onboarding": ["seen": true]]
    try JSONSerialization.data(withJSONObject: data).write(to: config)
    try fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: config.path)
    let process = Process()
    process.executableURL = executable
    process.arguments = ["preview", "latest"]
    process.environment = env
    let log = output.appendingPathComponent(disabled ? "disabled-app.log" : "default-app.log")
    fm.createFile(atPath: log.path, contents: nil, attributes: [.posixPermissions: 0o600])
    let handle = try FileHandle(forWritingTo: log); logs.append(handle)
    process.standardOutput = handle; process.standardError = handle
    try process.run(); owned = process
    let until = Date().addingTimeInterval(try remaining(15))
    var ready = false
    while Date() < until {
        if (try? window()) != nil { ready = true; break }
        try require(process.isRunning, "Preview exited during startup")
        Thread.sleep(forTimeInterval: 0.2)
    }
    try require(ready, "Preview window startup timeout")
    // Same identity check as the existing installed-app fixture: mapped executable,
    // not merely a PID or CLI help result. No scan of unrelated process contents.
    let mapped = try run("/usr/sbin/lsof", ["-nP", "-a", "-p", String(process.processIdentifier), "-d", "txt", "-Fn"])
    try require(mapped.components(separatedBy: "\n").contains("n" + executable.resolvingSymlinksInPath().path), "PID does not map installed executable")
    var launches = report["launches"] as? [[String: Any]] ?? []
    launches.append(["pid": process.processIdentifier, "executable": executable.path,
        "arguments": process.arguments!, "explicitFalse": disabled, "mappedExecutableVerified": true])
    report["launches"] = launches
    try focusAndSize(1120)
}
var failure: Error?
do {
    try require(output.path.hasPrefix(runner.path + "/") && app.path.hasPrefix(NSHomeDirectory() + "/Applications/Apparent-Storage-"), "Paths escape disposable CI fixture")
    try require(!fm.fileExists(atPath: output.path), "Evidence directory must be fresh")
    try fm.createDirectory(at: output, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    let home = runner.appendingPathComponent("ci-" + String(UUID().uuidString.prefix(8)))
    try fm.createDirectory(at: home, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    fixture = home
    let captures = home.appendingPathComponent("captures")
    try fm.createDirectory(at: captures, withIntermediateDirectories: false)
    let executable = app.appendingPathComponent("Contents/MacOS/apparent")
    try require(fm.isExecutableFile(atPath: executable.path), "Missing installed executable")
    _ = try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app.path])
    report["sourceCommit"] = try run("/usr/bin/git", ["rev-parse", "HEAD"])
    report["executableSHA256"] = try run("/usr/bin/shasum", ["-a", "256", executable.path])
    for key in ["HOME", "CFFIXED_USER_HOME", "TMPDIR", "TMP", "TEMP"] { env[key] = home.path }
    env["APPARENT_CONFIG"] = home.appendingPathComponent("config.json").path
    env["APPARENT_CAPTURES_DIR"] = captures.path
    env["XDG_CONFIG_HOME"] = home.appendingPathComponent(".config").path
    env["APPARENT_NO_ONBOARDING"] = "1"
    env.removeValue(forKey: "ELECTRON_RUN_AS_NODE")
    try require(home.appendingPathComponent(".config/apparent/agent-control/capture.sock").path.utf8.count <= 103,
                "Fixture control socket path exceeds macOS limit")
    try require(try run("/usr/bin/osascript", ["-e", "tell application \"System Events\" to get UI elements enabled"]) == "true", "Runner Accessibility preflight missing; do not edit TCC")
    let older = "older fixture — selected.apparent"
    let newer = "newer fixture.apparent"
    // Production generator supplies every bundle input. Only creation timestamps
    // are fixed so history ordering is deterministic (not directory mtime).
    for (index, name) in [older, newer].enumerated() {
        let bundle = captures.appendingPathComponent(name)
        _ = try run(executable.path, ["demo", "--out", bundle.path], cap: 25)
        let manifest = bundle.appendingPathComponent("apparent.json")
        var json = try JSONSerialization.jsonObject(with: Data(contentsOf: manifest)) as! [String: Any]
        try require(json["createdAtMs"] != nil, "Unknown manifest creation-time schema")
        json["createdAtMs"] = 1700000000000 + index * 1000
        try JSONSerialization.data(withJSONObject: json).write(to: manifest)
    }
    try launch(executable, disabled: false)
    try require(try window().bounds.width >= 600, "Wide fixture is not wide")
    try copyCheck("Copy capture ID", expected: newer, caseName: "wide-newest")
    try click("Older")
    try copyCheck("Copy capture ID", expected: older, caseName: "older-selected")
    try click("Newer")
    try copyCheck("Copy capture ID", expected: newer, caseName: "newer-selected")
    try copyCheck("Copy capture ID", expected: newer, caseName: "repeated-fresh-dispatch")
    try focusAndSize(560)
    try require(try window().bounds.width < 600, "Compact fixture did not resize below 600px")
    try copyCheck("Copy ID", expected: newer, caseName: "compact")
    try stop()
    try launch(executable, disabled: true)
    let (_, boxes) = try screenshot("explicit-false")
    try require(!boxes.contains { $0.bounds.midY > 0.86 && ($0.text.contains("Copy capture ID") || $0.text.contains("Copy ID")) }, "Explicit false still shows ID action")
    _ = try locate("Copy", in: boxes)
    report["explicitFalse"] = ["idActionAbsent": true, "ordinaryCopyPresent": true]
    report["passed"] = true
    report["nativeProof"] = "Executed installed Rust GPUI on GitHub-hosted macOS; inspect PNG and JSON artifacts"
} catch { failure = error; report["error"] = String(describing: error) }
// Retain failure pixels when possible, without widening to another window/screen.
if failure != nil && owned?.isRunning == true && deadline.timeIntervalSinceNow > 0 {
    _ = try? screenshot("failure-owned-window")
}
do { try stop(); report["ownedProcessStopped"] = true }
catch { failure = failure ?? error; report["passed"] = false; report["cleanupError"] = String(describing: error) }
for handle in logs { try? handle.close() }
if let home = fixture {
    if failure == nil {
        do { try fm.removeItem(at: home); report["fixtureRemoved"] = true }
        catch { failure = error; report["passed"] = false; report["cleanupError"] = String(describing: error) }
    } else { report["retainedFixture"] = home.path }
}
if fm.fileExists(atPath: output.path) {
    do {
        try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
            .write(to: output.appendingPathComponent("copy-capture-id.json"))
    } catch { fputs("Could not retain report: \(error)\n", stderr); exit(1) }
}
if let error = failure { fputs("Native Copy ID fixture failed: \(error)\n", stderr); exit(1) }
print("Native Copy ID evidence: \(output.path)")

// Owned-window input/inspection for the synthetic hosted Mac public-share case.
// No TCC edits, personal Mac access, guessed screen coordinates or publication.
import AppKit
import Vision
import ApplicationServices
import Foundation

func check(_ ok: Bool, _ message: String) throws {
    if !ok { throw NSError(domain: message, code: 1) }
}
let args = CommandLine.arguments
let env = ProcessInfo.processInfo.environment
let fm = FileManager.default
func perform() throws {
    try check(env["GITHUB_ACTIONS"] == "true" && env["RUNNER_ENVIRONMENT"] == "github-hosted" && env["APPARENT_CONNECT_NATIVE_CI"] == "1", "Hosted synthetic proof only")
    try check(args.count >= 2, "Missing operation")
    // Establish AppKit's WindowServer connection in every driver invocation.
    _ = NSApplication.shared
    if args[1] == "scene" {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        let rect = NSScreen.main!.frame
        let window = NSWindow(contentRect: rect, styleMask: [.borderless], backing: .buffered, defer: false)
        window.backgroundColor = NSColor(calibratedRed: 0.17, green: 0.12, blue: 0.34, alpha: 1)
        let label = NSTextField(labelWithString: "SYNTHETIC · Apparent public link acceptance")
        label.textColor = .white; label.font = .systemFont(ofSize: 34)
        label.frame = NSRect(x: 70, y: rect.height / 2, width: rect.width - 140, height: 80)
        window.contentView!.addSubview(label)
        window.makeKeyAndOrderFront(nil); app.activate(ignoringOtherApps: true)
        app.run(); return
    }
    if args[1] == "clipboard" {
        if args.count == 3 { NSPasteboard.general.clearContents(); try check(NSPasteboard.general.setString(args[2], forType: .string), "Clipboard seed failed") }
        else { print(NSPasteboard.general.string(forType: .string) ?? "") }
        return
    }
    try check(args.count == 5 && ["inspect", "click"].contains(args[1]), "Expected operation PID label screenshot")
    let pid = Int32(args[2])!
    let label = args[3]
    let output = URL(fileURLWithPath: args[4]).standardizedFileURL
    let root = URL(fileURLWithPath: env["RUNNER_TEMP"]!).resolvingSymlinksInPath()
    try check(output.path.hasPrefix(root.path + "/") && !fm.fileExists(atPath: output.path), "Fresh private runner evidence required")
    guard let app = NSRunningApplication(processIdentifier: pid) else { throw NSError(domain: "Owned app absent", code: 1) }
    let installed = URL(fileURLWithPath: env["APPARENT_APP_DIR"] ?? "/invalid").standardizedFileURL
    try check(installed.path.hasPrefix("/Users/runner/Applications/Apparent-Connect-"), "Unexpected installed app")
    try check(app.executableURL?.resolvingSymlinksInPath().path.hasPrefix(installed.path + "/Contents/") == true, "Target is outside the owned installed app")
    if args[1] == "click" { _ = app.activate(options: [.activateIgnoringOtherApps]); RunLoop.main.run(until: Date().addingTimeInterval(0.2)) }
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    var matches: [(CGWindowID, CGRect, CGRect, URL)] = []
    for row in rows where (row[kCGWindowOwnerPID as String] as? Int32) == pid {
        guard let id = row[kCGWindowNumber as String] as? UInt32,
              let dict = row[kCGWindowBounds as String] as? [String: Any],
              let bounds = CGRect(dictionaryRepresentation: dict as CFDictionary), bounds.width > 100, bounds.height > 20 else { continue }
        let image = output.deletingPathExtension().appendingPathExtension("\(id).png")
        let shot = Process(); shot.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        shot.arguments = ["-x", "-o", "-l", String(id), image.path]
        try shot.run()
        let until = Date().addingTimeInterval(3)
        while shot.isRunning && Date() < until { Thread.sleep(forTimeInterval: 0.05) }
        if shot.isRunning { shot.terminate(); throw NSError(domain: "Owned screenshot timeout", code: 1) }
        try check(shot.terminationStatus == 0, "Owned screenshot failed")
        let request = VNRecognizeTextRequest(); request.recognitionLevel = .accurate; request.usesLanguageCorrection = false
        try VNImageRequestHandler(url: image).perform([request])
        for item in request.results ?? [] {
            guard let text = item.topCandidates(1).first, let range = text.string.range(of: label, options: [.caseInsensitive]), let box = try text.boundingBox(for: range) else { continue }
            matches.append((id, bounds, box.boundingBox, image))
        }
    }
    try check(matches.count == 1, "Expected one owned visible text target: \(label); got \(matches.count)")
    let (id, bounds, box, image) = matches[0]
    try fm.copyItem(at: image, to: output)
    if args[1] == "click" {
        let fresh = CGWindowListCopyWindowInfo([.optionIncludingWindow], id) as? [[String: Any]] ?? []
        try check(fresh.count == 1 && (fresh[0][kCGWindowOwnerPID as String] as? Int32) == pid, "Input window ownership changed")
        let freshBounds = CGRect(dictionaryRepresentation: fresh[0][kCGWindowBounds as String] as! CFDictionary)
        try check(freshBounds == bounds && CGPreflightPostEventAccess(), "Bounds changed or native input permission absent")
        let point = CGPoint(x: bounds.minX + box.midX * bounds.width, y: bounds.minY + (1 - box.midY) * bounds.height)
        // Accessibility element lookup is not mouse hit-testing: a custom-drawn
        // nonactivating panel need not expose an AX element at its visible button.
        // AppKit uses the actual mouse-down rules, including transparent pixels
        // and ignoresMouseEvents. Require the exact inspected window, not merely
        // another window belonging to the same application.
        // https://developer.apple.com/documentation/appkit/nswindow/windownumber(at:belowwindowwithwindownumber:)
        let appKitPoint = NSPoint(x: point.x, y: CGDisplayBounds(CGMainDisplayID()).height - point.y)
        let beforeHit = NSWindow.windowNumber(at: appKitPoint, belowWindowWithWindowNumber: 0)
        let pointerBefore = NSEvent.mouseLocation
        // Native controls intentionally ignore input while the pointer is outside
        // their shape. Move without pressing, then allow their pointer poll to run.
        CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left)!.post(tap: .cghidEventTap)
        // A hosted display-link tick is not guaranteed within a fixed 300 ms.
        // Observe readiness with no repeated input; never click a fallback target.
        let hitDeadline = Date().addingTimeInterval(2)
        repeat {
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            let pointer = NSEvent.mouseLocation
            if hypot(pointer.x - appKitPoint.x, pointer.y - appKitPoint.y) <= 2,
               NSWindow.windowNumber(at: appKitPoint, belowWindowWithWindowNumber: 0) == Int(id) { break }
        } while Date() < hitDeadline
        let moved = CGWindowListCopyWindowInfo([.optionIncludingWindow], id) as? [[String: Any]] ?? []
        try check(moved.count == 1 && (moved[0][kCGWindowOwnerPID as String] as? Int32) == pid,
                  "Input window ownership changed after pointer movement")
        let movedBounds = CGRect(dictionaryRepresentation: moved[0][kCGWindowBounds as String] as! CFDictionary)
        try check(movedBounds == bounds && CGPreflightPostEventAccess(), "Bounds changed or input permission lost after pointer movement")
        let hitWindow = NSWindow.windowNumber(at: appKitPoint, belowWindowWithWindowNumber: 0)
        let pointerAfter = NSEvent.mouseLocation
        let diagnostic: [String: Any] = ["expectedWindow": id, "beforeHit": beforeHit, "afterHit": hitWindow,
            "windowBounds": [bounds.minX, bounds.minY, bounds.width, bounds.height],
            "ocrBounds": [box.minX, box.minY, box.width, box.height],
            "quartzPoint": [point.x, point.y], "appKitPoint": [appKitPoint.x, appKitPoint.y],
            "pointerBefore": [pointerBefore.x, pointerBefore.y],
            "pointerAfter": [pointerAfter.x, pointerAfter.y]]
        let data = try JSONSerialization.data(withJSONObject: diagnostic, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
        try check(hypot(pointerAfter.x - appKitPoint.x, pointerAfter.y - appKitPoint.y) <= 2,
                  "Pointer did not reach the validated target")
        try check(hitWindow == Int(id), "Mouse-down target differs from owned window (expected \(id), hit \(hitWindow))")
        for type in [CGEventType.leftMouseDown, .leftMouseUp] {
            CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left)!.post(tap: .cghidEventTap)
            RunLoop.main.run(until: Date().addingTimeInterval(0.08))
        }
    }
    print("Owned target verified")
}
do { try perform() } catch { fputs("Native public-share driver: \(error)\n", stderr); exit(1) }

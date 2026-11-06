// Compile with the complete production Export.swift and Media.swift, without the app main.
// File-only native proof: no screen, microphone, permission, network or installed app use.
import AppKit

struct HelperError: LocalizedError {
  let errorDescription: String?
  init(_ message: String) { errorDescription = message }
}
func emit(_ object: Any) {}

@main struct InkRGBProof {
  static func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw HelperError(message) }
  }

  static func ink(_ colors: Bool) throws -> ExportPlan.Ink {
    let color: (String) -> String = { colors ? ",\"rgb\":\($0)" : "" }
    let json = """
    {"k":1,"pinScale":1,"strokes":[{"snaps":[{"t":0,"poly":[0.1,0.6,0.4,0.6,0.4,0.7,0.1,0.7]}],"fadeAt":5000,"fadeMs":500\(color("[17,203,81]"))}],
    "pins":[{"x":0.5,"y":0.4,"n":1,"t":0,"fadeAt":5000,"fadeMs":500\(color("[0,0,255]"))}],
    "lasers":[{"frames":[{"t":0,"pts":[0.8,0.2,1,0.8,0.5,1]},{"t":2000,"pts":[0.8,0.2,1,0.8,0.5,1]}]\(color("[17,203,81]"))}],"ripples":[],"trails":[]}
    """
    return try JSONDecoder().decode(ExportPlan.Ink.self, from: Data(json.utf8))
  }

  static func render(_ ink: ExportPlan.Ink, at root: URL, name: String) throws -> [UInt8] {
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    let ctx = CGContext(data: nil, width: 320, height: 200, bitsPerComponent: 8,
      bytesPerRow: 320 * 4, space: space,
      bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.setFillColor(CGColor(red: 0.08, green: 0.08, blue: 0.08, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: 320, height: 200))
    ctx.translateBy(x: 0, y: 200); ctx.scaleBy(x: 1, y: -1)
    let view = ExportView(sourceW: 320, sourceH: 200, rect: [0,0,1,1], outW: 320, outH: 200)
    ExportInk.draw(ctx, t: 1000, view: view, ink: ink)
    let bytes = Array(UnsafeBufferPointer(start: ctx.data!.assumingMemoryBound(to: UInt8.self), count: 320 * 200 * 4))
    try Data(bytes).write(to: root.appendingPathComponent("\(name).rgba"), options: .withoutOverwriting)
    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    try rep.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent("\(name).png"), options: .withoutOverwriting)
    return bytes
  }

  static func main() throws {
    guard CommandLine.arguments.count >= 2 else { throw HelperError("provide a fresh evidence directory") }
    let root = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    let legacy = try render(ink(false), at: root, name: "legacy")
    if CommandLine.arguments.contains("--legacy-only") { return }
    let colored = try render(ink(true), at: root, name: "colored")
    try require(legacy != colored, "recorded RGB must change rendered output")
    let pixels = stride(from: 0, to: colored.count, by: 4)
    try require(pixels.filter { Array(colored[$0..<$0+4]) == [17,203,81,255] }.count > 100,
      "plain stroke must retain exact encoded RGB")
    try require(pixels.filter { colored[$0+2] > 200 && colored[$0] < 30 && colored[$0+1] < 30 }.count > 50,
      "pin must use its independently recorded blue")
    // The right-hand laser region must use the recorded hue, while keeping its white core.
    let laserPixels = pixels.filter { ($0 / 4) % 320 > 240 }
    try require(laserPixels.filter { Int(colored[$0+1]) > Int(colored[$0]) + 30 && Int(colored[$0+1]) > Int(colored[$0+2]) + 30 }.count > 30,
      "laser glow must use recorded hue")
    for invalid in ["[]", "[0,0]", "[0,0,0,0]", "[-1,0,0]", "[256,0,0]", "[0.5,0,0]", "\"coral\""] {
      let json = "{\"snaps\":[],\"fadeAt\":0,\"fadeMs\":1,\"rgb\":\(invalid)}"
      do {
        _ = try JSONDecoder().decode(ExportPlan.Stroke.self, from: Data(json.utf8))
        throw HelperError("accepted invalid RGB \(invalid)")
      } catch is DecodingError {}
    }
    print("PASS native RGB decoding, legacy default, exact stroke, independent pin and laser rendering")
  }
}

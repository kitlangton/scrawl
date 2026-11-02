// Run from the repository root: swift capture/desktop/scripts/native/make-menu-bar-light.swift
// Preserve the approved raster's glyph coverage; change only its two brand colors.
import AppKit

let source = "capture/desktop/assets/menubar-icon@2x.png"
let destination = "capture/desktop/assets/menubar-light@2x.png"
let original = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: source)))!
let light = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: original.pixelsWide,
    pixelsHigh: original.pixelsHigh, bitsPerSample: 8, samplesPerPixel: 3, hasAlpha: false,
    isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
// Paper and the website's dark purple focus/selection color.
let paper: [CGFloat] = [250, 249, 246]
let purple: [CGFloat] = [122, 88, 199]
for y in 0..<original.pixelsHigh {
    for x in 0..<original.pixelsWide {
        var pixel = [UInt](repeating: 0, count: 4)
        original.getPixel(&pixel, atX: x, y: y)
        // Carbon red = 2, Lilac red = 183; preserve antialiased edges.
        let coverage = min(1, max(0, (CGFloat(pixel[0]) - 2) / 181))
        var rgb = zip(paper, purple).map { UInt(($0 + ($1 - $0) * coverage).rounded()) }
        light.setPixel(&rgb, atX: x, y: y)
    }
}
if CommandLine.arguments.contains("--check") {
    let saved = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: destination)))!
    precondition(saved.pixelsWide == 36 && saved.pixelsHigh == 36)
    var background = 0, solid = 0, edge = 0
    for y in 0..<36 {
        for x in 0..<36 {
            var expected = [UInt](repeating: 0, count: 4)
            var actual = expected, sourcePixel = expected
            light.getPixel(&expected, atX: x, y: y)
            saved.getPixel(&actual, atX: x, y: y)
            original.getPixel(&sourcePixel, atX: x, y: y)
            precondition(Array(expected.prefix(3)) == Array(actual.prefix(3)),
                "Approved shape/color must be preserved")
            let red = sourcePixel[0]
            if red <= 2 { background += 1 }
            else if red >= 183 { solid += 1 }
            else { edge += 1 }
        }
    }
    precondition(background > 500 && solid > 200 && edge > 20)
    func luminance(_ rgb: [CGFloat]) -> CGFloat {
        let linear = rgb.map { value -> CGFloat in
            let v = value / 255
            return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
        }
        return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
    }
    let contrast = (luminance(paper) + 0.05) / (luminance(purple) + 0.05)
    precondition(contrast >= 3, "Small graphical icon needs at least 3:1 internal contrast")
    print("PASS: approved shape, antialiased edges, brand colors; light contrast \(contrast):1")
} else {
    let srgb = light.retagging(with: .sRGB)!
    try srgb.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: destination))
}

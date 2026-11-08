import AppKit
import CoreText
import SwiftUI

/// Brand roles shared only by onboarding permission guidance.
/// Popup silhouettes and frog artwork/motion are intentional user-approved exceptions.
enum OnboardingBrand {
  static let paper = NSColor(srgbRed: 245.0/255.0, green: 240.0/255.0, blue: 232.0/255.0, alpha: 1)
  static let carbon = NSColor(srgbRed: 2.0/255.0, green: 1.0/255.0, blue: 2.0/255.0, alpha: 1)
  static let lilac = Color(red: 183.0/255.0, green: 165.0/255.0, blue: 216.0/255.0)
  static let signal = Color(red: 122.0/255.0, green: 88.0/255.0, blue: 199.0/255.0)
  static let success = Color(red: 20.0/255.0, green: 107.0/255.0, blue: 67.0/255.0)
  static let failure = Color(red: 180.0/255.0, green: 35.0/255.0, blue: 24.0/255.0)
  static let backgroundNS = NSColor(name: nil) { appearance in
    appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? carbon : paper
  }
  static let foregroundNS = NSColor(name: nil) { appearance in
    appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? paper : carbon
  }
  static var background: Color { Color(nsColor: backgroundNS) }
  static var foreground: Color { Color(nsColor: foregroundNS) }
  private static var fontURL: URL {
    let source = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent().appending(path: "shared/assets/fonts/HostGrotesk-Variable.ttf")
    return Bundle.main.url(forResource: "HostGrotesk-Variable", withExtension: "ttf") ?? source
  }
  private static let fontDescriptor: CTFontDescriptor? = {
    (CTFontManagerCreateFontDescriptorsFromURL(fontURL as CFURL) as? [CTFontDescriptor])?.first
  }()
  static func platformFont(_ size: CGFloat, weight: Int = 400) -> CTFont {
    // macOS does not necessarily expose a variable font's named instances by name.
    // Select the bundled face and its actual OpenType weight axis instead.
    let base = fontDescriptor ?? CTFontDescriptorCreateWithNameAndSize("HostGrotesk-Light" as CFString, size)
    let varied = CTFontDescriptorCreateCopyWithVariation(base, NSNumber(value: 0x77676874), CGFloat(weight))
    return CTFontCreateWithFontDescriptor(varied, size, nil)
  }
  static func font(_ size: CGFloat, weight: Int = 400) -> Font {
    Font(platformFont(size, weight: weight))
  }
  static func loadFont() {
    CTFontManagerRegisterFontsForURL(fontURL as CFURL, .process, nil)
  }

  struct ActionStyle: ButtonStyle {
    var primary = false
    @Environment(\.isEnabled) private var enabled
    @Environment(\.isFocused) private var focused
    func makeBody(configuration: Configuration) -> some View {
      configuration.label.font(OnboardingBrand.font(18, weight: 500))
        .padding(.horizontal, 16).padding(.vertical, 8)
        .frame(minWidth: 44, minHeight: 44)
        .foregroundStyle(primary ? Color(nsColor: OnboardingBrand.paper) : OnboardingBrand.foreground)
        .background(primary ? OnboardingBrand.signal : OnboardingBrand.background)
        .overlay(Rectangle().stroke(focused ? OnboardingBrand.signal : OnboardingBrand.foreground, lineWidth: focused ? 2 : 1))
        .opacity(enabled ? 1 : 0.45)
        .contentShape(Rectangle())
    }
  }
}

import SwiftUI
import VisionKit

/// Pairing: the Mac prints a setup link and its QR code; the Camera opens it in Scrawl, or it's
/// scanned or pasted here.
struct PairSection: View {
  @Environment(AppModel.self) private var model
  @State private var scanning = false
  @State private var problem: String?

  var body: some View {
    Section {
      VStack(alignment: .leading, spacing: 10) {
        Text("Pair with your Mac").font(.headline)
        Text("On the Mac with Scrawl, run this in a terminal, then scan the QR code with this iPhone's Camera:")
          .font(.subheadline)
          .foregroundStyle(.secondary)
        Text("scrawl link")
          .font(.body.monospaced())
          .padding(.horizontal, 10)
          .padding(.vertical, 6)
          .background(.quaternary, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
          .textSelection(.enabled)
      }
      .padding(.vertical, 4)
      if DataScannerViewController.isSupported {
        Button("Scan Code", systemImage: "qrcode.viewfinder") { scanning = true }
      }
      PasteButton(payloadType: String.self) { texts in
        let pasted = texts.joined(separator: " ")
        Task { @MainActor in
          problem = model.paste(pasted) ? nil : "That isn't a Scrawl link. It starts with scrawl:// and has a token."
        }
      }
      if let problem {
        Text(problem).font(.footnote).foregroundStyle(.orange)
      }
    } footer: {
      Text("Your iPhone and Mac need to be on the same Wi‑Fi or tailnet. Scrawl on the Mac (the app, or scrawl serve) builds each capture's bundle and queues it into an OpenCode session.")
    }
    .sheet(isPresented: $scanning) {
      NavigationStack {
        QRScanner { text in
          if let url = URL(string: text), model.open(url) {
            scanning = false
          }
        }
        .ignoresSafeArea()
        .navigationTitle("Scan the code")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .cancellationAction) {
            Button("Cancel") { scanning = false }
          }
        }
      }
    }
  }
}

/// VisionKit's live scanner, for QR codes.
private struct QRScanner: UIViewControllerRepresentable {
  let found: (String) -> Void

  func makeUIViewController(context: Context) -> DataScannerViewController {
    let scanner = DataScannerViewController(
      recognizedDataTypes: [.barcode(symbologies: [.qr])],
      qualityLevel: .balanced,
      recognizesMultipleItems: false,
      isHighFrameRateTrackingEnabled: false,
      isHighlightingEnabled: true
    )
    scanner.delegate = context.coordinator
    return scanner
  }

  func updateUIViewController(_ scanner: DataScannerViewController, context _: Context) {
    if !scanner.isScanning { try? scanner.startScanning() }
  }

  func makeCoordinator() -> Coordinator { Coordinator(found: found) }

  @MainActor
  final class Coordinator: NSObject, DataScannerViewControllerDelegate {
    let found: (String) -> Void

    init(found: @escaping (String) -> Void) {
      self.found = found
    }

    func dataScanner(_: DataScannerViewController, didAdd items: [RecognizedItem], allItems _: [RecognizedItem]) {
      for case let .barcode(code) in items {
        if let text = code.payloadStringValue { found(text) }
      }
    }
  }
}

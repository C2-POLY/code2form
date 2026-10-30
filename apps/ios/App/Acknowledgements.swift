// The credits: Code2Form's licence and source, the libraries the app is
// built with and their licences (Acknowledgements.txt, written by
// scripts/ios-acknowledgements.sh), and the fonts' licence, which must
// travel with them.

import SwiftUI

struct AcknowledgementsButton: View {
    var body: some View {
        Button {
            Cover.present { close in AcknowledgementsView(close: close) }
        } label: {
            Label("Acknowledgements", systemImage: "info.circle")
        }
    }
}

struct AcknowledgementsView: View {
    let close: () -> Void

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(version).font(.headline)
                    Text(Self.read("Acknowledgements.txt"))
                    Text("Liberation fonts").font(.headline)
                    Text(Self.read("fonts/Liberation-2.00.1/LICENSE"))
                }
                .font(.footnote)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
            }
            .navigationTitle("Acknowledgements")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: close)
                }
            }
        }
    }

    private var version: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let short = info["CFBundleShortVersionString"] as? String ?? "?"
        let build = info["CFBundleVersion"] as? String ?? "?"
        return "Code2Form \(short) (\(build))"
    }

    /// A text file of the app's bundle.
    private static func read(_ path: String) -> String {
        guard let url = Bundle.main.resourceURL?.appendingPathComponent(path),
              let text = try? String(contentsOf: url, encoding: .utf8) else { return "" }
        return text
    }
}

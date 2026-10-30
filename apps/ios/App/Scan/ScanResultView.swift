// The rebuilt scan in 3D with its size, already saved in Files, and sent
// on to the Mac from here.

import SwiftUI

import Code2FormCore

struct ScanResultView: View {
    let scan: Scan
    let close: () -> Void

    var body: some View {
        ZStack(alignment: .topLeading) {
            if scan.rendering.error == nil, scan.rendering.bounds != nil {
                ModelView(rendering: scan.rendering)
                    .ignoresSafeArea()
            }
            VStack(alignment: .leading, spacing: 4) {
                if let bounds = scan.rendering.bounds {
                    let size = zip(bounds.max, bounds.min).map { $0 - $1 }
                    Text(String(format: "%.1f × %.1f × %.1f mm", size[0], size[1], size[2]))
                        .font(.headline)
                    Text("\(scan.rendering.triangles) triangles")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                } else if let error = scan.rendering.error {
                    Label(error.message, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
                }
                Text("Saved in Code2Form › Scans as \(scan.file.lastPathComponent)")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                HStack {
                    ShareLink(item: scan.file) {
                        Label("Send to the Mac", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(.borderedProminent)
                    Button("Done", action: close)
                        .buttonStyle(.bordered)
                }
                .padding(.top, 8)
            }
            .padding(12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            .padding()
        }
    }
}

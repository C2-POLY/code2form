// A design: its model in 3D, with its size, or the error that stopped
// the program.

import SwiftUI

import Code2FormCore

struct DesignView: View {
    let source: String
    let url: URL?

    @State private var rendering: Rendering?
    @State private var failure: String?
    /// The design's folder cannot be read: `include`, `use`, `import()`
    /// and its circuit need the user to pick it once.
    @State private var folderClosed = false
    @State private var pickingFolder = false
    /// Bumped to render again once the folder is open.
    @State private var attempt = 0

    var body: some View {
        ZStack(alignment: .topLeading) {
            if let rendering, rendering.error == nil, rendering.bounds != nil {
                ModelView(rendering: rendering)
                    .ignoresSafeArea()
            } else {
                Color(red: 0.16, green: 0.17, blue: 0.19).ignoresSafeArea()
            }
            overlay
                .padding()
        }
        .task(id: "\(attempt)\(source)") { await render() }
        .fileImporter(isPresented: $pickingFolder, allowedContentTypes: [.folder]) { result in
            if case .success(let folder) = result {
                FolderAccess.remember(folder)
                attempt += 1
            }
        }
    }

    /// Asks for the folder when the program missed something beside it.
    @ViewBuilder
    private func folderButton(_ rendering: Rendering) -> some View {
        if folderClosed, rendering.error != nil || !rendering.warnings.isEmpty {
            Button {
                pickingFolder = true
            } label: {
                Label("Allow access to the folder", systemImage: "folder.badge.questionmark")
            }
            .padding(.top, 4)
            Text("To read the files it includes, imports or wires.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var overlay: some View {
        if let failure {
            card {
                Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.red)
            }
        } else if let rendering {
            if let error = rendering.error {
                card {
                    Label("The program stopped", systemImage: "exclamationmark.triangle")
                        .font(.headline)
                    Text(error.line.map { "Line \($0): \(error.message)" } ?? error.message)
                        .font(.body.monospaced())
                    folderButton(rendering)
                }
            } else if let bounds = rendering.bounds {
                let size = zip(bounds.max, bounds.min).map { $0 - $1 }
                card {
                    Text(String(format: "%.1f × %.1f × %.1f mm", size[0], size[1], size[2]))
                        .font(.headline)
                    Text("\(rendering.triangles) triangles · \(String(format: "%.0f", rendering.volume)) mm³")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    ForEach(rendering.warnings, id: \.self) { Text($0).font(.footnote).foregroundStyle(.orange) }
                    folderButton(rendering)
                }
            } else {
                card { Text("The model is empty") }
            }
        } else {
            card { ProgressView("Rendering…") }
        }
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4, content: content)
            .padding(12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
    }

    private func render() async {
        let url = url
        let request = Request(source: source, path: url?.path ?? "untitled.c2f", parameters: [])
        // The engine blocks: run it away from the main actor.
        let (result, closed) = await Task.detached { () -> (Result<Rendering, Error>, Bool) in
            FolderAccess.reading(around: url) {
                let folder = url?.deletingLastPathComponent()
                let closed = folder.map { !FolderAccess.canRead($0) } ?? false
                return (Result { try Engines.make().render(request: request) }, closed)
            }
        }.value
        folderClosed = closed
        switch result {
        case .success(let rendering): self.rendering = rendering
        case .failure(let error): failure = "\(error)"
        }
    }
}

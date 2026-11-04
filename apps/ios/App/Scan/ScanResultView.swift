// The rebuilt scan in 3D with its size, already saved in Files, and sent
// on to the Mac from here, or saved into an object's project.

import SwiftUI
import UniformTypeIdentifiers

import Code2FormCore

struct ScanResultView: View {
    let scan: Scan
    let close: () -> Void

    /// Picking the folder of the project, or of the projects.
    @State private var picking = false
    /// The projects in the folder picked, to choose from.
    @State private var choices: [ProjectSummary] = []
    @State private var folder: URL?
    /// A folder without a project: the name of the one to create in it.
    @State private var naming = false
    @State private var newName = ""
    /// What became of the scan.
    @State private var saved: String?
    @State private var failure: String?

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
                if let saved {
                    Label("In the project \(saved)", systemImage: "checkmark.circle")
                        .font(.footnote)
                        .foregroundStyle(.green)
                }
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
                HStack {
                    ShareLink(item: scan.file) {
                        Label("Send to the Mac", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(.borderedProminent)
                    Button {
                        picking = true
                    } label: {
                        Label("Save to a Project…", systemImage: "folder.badge.plus")
                    }
                    .buttonStyle(.bordered)
                    Button("Done", action: close)
                        .buttonStyle(.bordered)
                }
                .padding(.top, 8)
            }
            .padding(12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            .padding()
        }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result {
                picked(url)
            }
        }
        .confirmationDialog("Which project?", isPresented: Binding(get: { !choices.isEmpty }, set: { if !$0 { choices = [] } })) {
            ForEach(choices, id: \.folder) { project in
                Button(project.name) { save(into: project) }
            }
        }
        .alert("A new project here", isPresented: $naming) {
            TextField("Its name", text: $newName)
            Button("Create") { create() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This folder holds no project: name the object's, and the scan goes in its scans folder.")
        }
    }

    /// A folder picked: a project, a folder of projects, or one to create
    /// a project in. Code2Form › Projects in iCloud Drive is shared with
    /// the studio on the Mac.
    private func picked(_ url: URL) {
        FolderAccess.remember(url)
        folder = url
        failure = nil
        let found = inside(url) { projectsIn(folder: url.path) }
        switch found.count {
        case 0:
            newName = ""
            naming = true
        case 1:
            save(into: found[0])
        default:
            choices = found
        }
    }

    private func save(into project: ProjectSummary) {
        guard let folder else { return }
        do {
            _ = try inside(folder) { try addScan(folder: project.folder, file: scan.file.path) }
            saved = project.name
        } catch {
            failure = error.localizedDescription
        }
    }

    private func create() {
        guard let folder, !newName.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        do {
            let project = try inside(folder) { try createProject(parent: folder.path, name: newName) }
            save(into: project)
        } catch {
            failure = error.localizedDescription
        }
    }

    /// Runs `work` with the folder the user picked open: iOS lets the app
    /// into it only then.
    private func inside<T>(_ folder: URL, _ work: () throws -> T) rethrows -> T {
        let opened = folder.startAccessingSecurityScopedResource()
        defer { if opened { folder.stopAccessingSecurityScopedResource() } }
        return try work()
    }
}

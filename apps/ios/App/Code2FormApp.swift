// Code2Form on iPhone and iPad: opens a design from Files and renders it
// with the engine.

import SwiftUI
import UniformTypeIdentifiers

import Code2FormCore

@main
struct Code2FormApp: App {
    var body: some Scene {
        DocumentGroup(viewing: Design.self) { file in
            DesignView(source: file.document.source, url: file.fileURL)
        }
    }
}

extension UTType {
    static let c2f = UTType(exportedAs: "com.c2poly.c2f")
    static let scad = UTType(importedAs: "org.openscad.scad")
}

/// A `.c2f` or `.scad` file: its program's text.
struct Design: FileDocument {
    static let readableContentTypes: [UTType] = [.c2f, .scad]

    var source: String

    init(configuration: ReadConfiguration) throws {
        guard let data = configuration.file.regularFileContents else {
            throw CocoaError(.fileReadCorruptFile)
        }
        // The reference accepts any bytes: invalid UTF-8 is replaced.
        source = String(decoding: data, as: UTF8.self)
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: Data(source.utf8))
    }
}

/// The engine, with the fonts and libraries the app carries.
enum Engines {
    static func make() throws -> Engine {
        let resources = Bundle.main.resourceURL!
        return try Engine(
            fonts: [resources.appendingPathComponent("fonts").path],
            libraries: [resources.appendingPathComponent("library").path]
        )
    }
}

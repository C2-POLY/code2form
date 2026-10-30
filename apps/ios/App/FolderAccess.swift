// The folders a design reads beside it: `include`, `use`, `import()` and
// its circuit. iOS lets an app read only the file it opened; a folder the
// user picks once is remembered as a bookmark and opened again at each
// render.

import Foundation

enum FolderAccess {
    private static let key = "folderBookmarks"

    /// Whether the app can list `folder` now.
    static func canRead(_ folder: URL) -> Bool {
        (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) != nil
    }

    /// Remembers `folder`, which the user just picked.
    static func remember(_ folder: URL) {
        guard folder.startAccessingSecurityScopedResource() else { return }
        defer { folder.stopAccessingSecurityScopedResource() }
        guard let bookmark = try? folder.bookmarkData() else { return }
        var bookmarks = UserDefaults.standard.dictionary(forKey: key) as? [String: Data] ?? [:]
        bookmarks[folder.standardizedFileURL.path] = bookmark
        UserDefaults.standard.set(bookmarks, forKey: key)
    }

    /// Opens the remembered folders that hold `file`, runs `work`, then
    /// closes them.
    static func reading<T>(around file: URL?, _ work: () throws -> T) rethrows -> T {
        let bookmarks = UserDefaults.standard.dictionary(forKey: key) as? [String: Data] ?? [:]
        let path = file?.standardizedFileURL.path ?? ""
        var opened: [URL] = []
        for (folder, bookmark) in bookmarks where path.hasPrefix(folder + "/") {
            var stale = false
            if let url = try? URL(resolvingBookmarkData: bookmark, bookmarkDataIsStale: &stale),
               url.startAccessingSecurityScopedResource() {
                opened.append(url)
            }
        }
        defer { opened.forEach { $0.stopAccessingSecurityScopedResource() } }
        return try work()
    }
}

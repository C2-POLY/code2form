// A screen over the whole app, presented by UIKit: the document launch
// screen, where the scan's and the credits' buttons live, presents no
// SwiftUI cover.

import SwiftUI
import UIKit

enum Cover {
    /// Shows the view `make` builds over the app; the view closes it by
    /// calling the closure it is given.
    @MainActor
    static func present<Content: View>(_ make: @escaping (@escaping () -> Void) -> Content) {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        guard var top = scenes.flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController else { return }
        while let shown = top.presentedViewController { top = shown }
        let host = UIHostingController<Content?>(rootView: nil)
        host.rootView = make { [weak host] in host?.dismiss(animated: true) }
        host.modalPresentationStyle = .fullScreen
        top.present(host, animated: true)
    }
}

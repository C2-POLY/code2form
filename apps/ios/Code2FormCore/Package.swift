// swift-tools-version:6.0
// The engine of Code2Form for the iPad app: crates/mobile, built by
// scripts/ios-xcframework.sh into Code2FormCore.xcframework, with the
// Swift bindings UniFFI writes into Sources/Code2FormCore. Run the script
// before building this package.

import PackageDescription

let package = Package(
    name: "Code2FormCore",
    platforms: [.iOS(.v18)],
    products: [
        .library(name: "Code2FormCore", targets: ["Code2FormCore"]),
    ],
    targets: [
        .binaryTarget(name: "Code2FormCoreFFI", path: "Code2FormCore.xcframework"),
        .target(name: "Code2FormCore", dependencies: ["Code2FormCoreFFI"]),
        .testTarget(name: "Code2FormCoreTests", dependencies: ["Code2FormCore"]),
    ],
    swiftLanguageModes: [.v5]
)

// swift-tools-version:6.0
import PackageDescription

// AIDEV-NOTE: run scripts/ffi.sh first; CAfterimage and AfterimageEngine are generated from the Rust core.
let package = Package(
    name: "Afterimage",
    platforms: [.macOS("26.0")],
    targets: [
        .target(name: "CAfterimage"),
        .target(name: "AfterimageEngine", dependencies: ["CAfterimage"], swiftSettings: [.swiftLanguageMode(.v5)],
                linkerSettings: [.unsafeFlags(["-L../build/lib"]), .linkedLibrary("afterimage_core"),
                                 .linkedFramework("Security"), .linkedFramework("SystemConfiguration")]),
        .target(name: "AfterimageKit", dependencies: ["AfterimageEngine"], swiftSettings: [.swiftLanguageMode(.v5)]),
        .executableTarget(name: "Afterimage", dependencies: ["AfterimageKit"], swiftSettings: [.swiftLanguageMode(.v5)]),
        .executableTarget(name: "afterimage-checks", dependencies: ["AfterimageKit"], swiftSettings: [.swiftLanguageMode(.v5)]),
        .testTarget(name: "AfterimageKitTests", dependencies: ["AfterimageKit"], swiftSettings: [.swiftLanguageMode(.v5)]),
    ]
)

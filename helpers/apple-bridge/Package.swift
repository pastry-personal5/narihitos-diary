// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "apple-bridge",
    platforms: [
        .macOS(.v26),
    ],
    products: [
        .executable(name: "apple-bridge", targets: ["AppleBridge"]),
    ],
    targets: [
        .target(name: "AppleBridgeProtocol"),
        .executableTarget(
            name: "AppleBridge",
            dependencies: ["AppleBridgeProtocol"]
        ),
        .testTarget(
            name: "AppleBridgeProtocolTests",
            dependencies: ["AppleBridgeProtocol"]
        ),
    ]
)

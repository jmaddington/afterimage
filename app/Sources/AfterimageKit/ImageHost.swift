import AfterimageEngine
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Apple image services for the engine. Only ever receives frames whose secrets are already frosted.
public final class ImageHost: HostServices, @unchecked Sendable {
    public var onChange: (@Sendable () -> Void)?
    public init() {}

    public func encode(rgba: Data, width: UInt32, height: UInt32, stride: UInt32, path: String, thumbPath: String) throws -> Encoded {
        try autoreleasepool {
            guard let full = Pixels.image(Bitmap(data: rgba, width: Int(width), height: Int(height), stride: Int(stride))) else {
                throw HostError.Failed(reason: "bad frame")
            }
            let main = Pixels.scaled(full, maxEdge: 2880)
            var bytes = try write(main, to: path, type: .heic, quality: 0.6)
            bytes += try write(Pixels.scaled(main, maxEdge: 720), to: thumbPath, type: .heic, quality: 0.55)
            return Encoded(width: UInt32(main.width), height: UInt32(main.height), bytes: bytes)
        }
    }

    public func modelImage(path: String, maxEdge: UInt32) throws -> Data {
        guard let i = Pixels.load(path, maxEdge: Int(maxEdge)) else { throw HostError.Failed(reason: "unreadable picture") }
        let d = NSMutableData()
        try write(i, into: CGImageDestinationCreateWithData(d, UTType.jpeg.identifier as CFString, 1, nil), quality: 0.8)
        return d as Data
    }

    public func libraryChanged() { onChange?() }

    private func write(_ image: CGImage, to path: String, type: UTType, quality: Double) throws -> UInt64 {
        let d = NSMutableData()
        try write(image, into: CGImageDestinationCreateWithData(d, type.identifier as CFString, 1, nil), quality: quality)
        // Created 0600 from the first byte rather than tightened afterwards.
        guard FileManager.default.createFile(atPath: path, contents: d as Data, attributes: [.posixPermissions: 0o600]) else {
            throw HostError.Failed(reason: "couldn't write the file")
        }
        return UInt64(d.length)
    }

    private func write(_ image: CGImage, into dest: CGImageDestination?, quality: Double) throws {
        guard let dest else { throw HostError.Failed(reason: "no encoder") }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { throw HostError.Failed(reason: "encoding failed") }
    }
}

import CoreGraphics
import Foundation
import ImageIO

public struct Bitmap: Sendable {
    public var data: Data
    public var width: Int
    public var height: Int
    public var stride: Int
}

public enum Pixels {
    static let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!
    static let rgba = CGImageAlphaInfo.premultipliedLast.rawValue

    public static func bitmap(_ image: CGImage) -> Bitmap? {
        let w = image.width, h = image.height, stride = w * 4
        guard w > 0, h > 0 else { return nil }
        var data = Data(count: stride * h)
        let ok = data.withUnsafeMutableBytes { p -> Bool in
            guard let c = CGContext(data: p.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: stride, space: sRGB, bitmapInfo: rgba) else { return false }
            c.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
            return true
        }
        return ok ? Bitmap(data: data, width: w, height: h, stride: stride) : nil
    }

    public static func image(_ b: Bitmap) -> CGImage? {
        guard b.width > 0, b.height > 0, b.stride >= b.width * 4, b.data.count >= b.stride * b.height,
              let p = CGDataProvider(data: b.data as CFData) else { return nil }
        return CGImage(width: b.width, height: b.height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: b.stride, space: sRGB,
                       bitmapInfo: CGBitmapInfo(rawValue: rgba), provider: p, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }

    public static func scaled(_ image: CGImage, maxEdge: Int) -> CGImage {
        let edge = max(image.width, image.height)
        guard edge > maxEdge else { return image }
        let k = Double(maxEdge) / Double(edge)
        let w = max(1, Int(Double(image.width) * k)), h = max(1, Int(Double(image.height) * k))
        guard let c = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: sRGB, bitmapInfo: rgba) else { return image }
        c.interpolationQuality = .high
        c.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        return c.makeImage() ?? image
    }

    /// Small greyscale fingerprint for "has anything changed". The menu bar is skipped: its clock always changes.
    /// AIDEV-NOTE: 128x80 so one typed character still moves a cell; a coarser grid misses typing.
    public static func thumbprint(_ image: CGImage) -> Data {
        let w = 128, h = 80
        let bar = min(image.height / 25, 80)
        guard let body = image.cropping(to: CGRect(x: 0, y: bar, width: image.width, height: image.height - bar)) else { return Data() }
        var out = Data(count: w * h)
        out.withUnsafeMutableBytes { p in
            guard let c = CGContext(data: p.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w,
                                    space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return }
            c.interpolationQuality = .high
            c.draw(body, in: CGRect(x: 0, y: 0, width: w, height: h))
        }
        return out
    }

    public static func load(_ path: String, maxEdge: Int? = nil) -> CGImage? {
        guard let s = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else { return nil }
        if let m = maxEdge {
            return CGImageSourceCreateThumbnailAtIndex(s, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: m,
                kCGImageSourceShouldCacheImmediately: true,
            ] as CFDictionary)
        }
        return CGImageSourceCreateImageAtIndex(s, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
    }
}

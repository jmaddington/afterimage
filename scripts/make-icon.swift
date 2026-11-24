// Draws the app icon: a bright disc and the cool ghost it leaves behind.
import AppKit
let out = CommandLine.arguments[1]
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
func draw(_ px: Int) -> Data {
  let S = CGFloat(px)
  let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
  let c = NSGraphicsContext.current!.cgContext
  let inset = S * 0.098
  let box = CGRect(x: inset, y: inset, width: S - 2 * inset, height: S - 2 * inset)
  let shape = CGPath(roundedRect: box, cornerWidth: box.width * 0.2237, cornerHeight: box.width * 0.2237, transform: nil)
  let sp = CGColorSpace(name: CGColorSpace.displayP3)!
  func col(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat, _ a: CGFloat = 1) -> CGColor { CGColor(colorSpace: sp, components: [r, g, b, a])! }
  c.saveGState()
  c.setShadow(offset: CGSize(width: 0, height: -S * 0.012), blur: S * 0.03, color: col(0, 0, 0, 0.35))
  c.addPath(shape); c.setFillColor(col(0.05, 0.06, 0.1)); c.fillPath()
  c.restoreGState()
  c.saveGState()
  c.addPath(shape); c.clip()
  c.drawLinearGradient(CGGradient(colorsSpace: sp, colors: [col(0.13, 0.15, 0.25), col(0.03, 0.035, 0.07)] as CFArray, locations: [0, 1])!, start: CGPoint(x: S / 2, y: box.maxY), end: CGPoint(x: S / 2, y: box.minY), options: [])
  func disc(_ x: CGFloat, _ y: CGFloat, _ r: CGFloat, _ stops: [CGColor]) {
    let p = CGPoint(x: box.minX + box.width * x, y: box.minY + box.height * y), R = box.width * r
    let locs = (0..<stops.count).map { CGFloat($0) / CGFloat(stops.count - 1) }
    c.drawRadialGradient(CGGradient(colorsSpace: sp, colors: stops as CFArray, locations: locs)!, startCenter: p, startRadius: 0, endCenter: p, endRadius: R, options: [])
  }
  // Ghosts fade as they drift from the source.
  disc(0.70, 0.36, 0.26, [col(0.30, 0.55, 1, 0.20), col(0.30, 0.55, 1, 0.16), col(0.3, 0.55, 1, 0)])
  disc(0.60, 0.44, 0.27, [col(0.38, 0.62, 1, 0.42), col(0.38, 0.62, 1, 0.34), col(0.38, 0.62, 1, 0)])
  c.setBlendMode(.plusLighter)
  disc(0.43, 0.57, 0.46, [col(1, 0.72, 0.35, 0.42), col(1, 0.55, 0.25, 0.12), col(1, 0.5, 0.2, 0)])
  c.setBlendMode(.normal)
  let R = box.width * 0.215, p = CGPoint(x: box.minX + box.width * 0.43, y: box.minY + box.height * 0.57)
  c.saveGState()
  c.addEllipse(in: CGRect(x: p.x - R, y: p.y - R, width: 2 * R, height: 2 * R)); c.clip()
  c.drawLinearGradient(CGGradient(colorsSpace: sp, colors: [col(1, 0.97, 0.88), col(1, 0.80, 0.46)] as CFArray, locations: [0, 1])!, start: CGPoint(x: p.x, y: p.y + R), end: CGPoint(x: p.x, y: p.y - R), options: [])
  c.restoreGState()
  c.restoreGState()
  NSGraphicsContext.restoreGraphicsState()
  return rep.representation(using: .png, properties: [:])!
}
for (n, px) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128), ("128x128@2x", 256), ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
  try! draw(px).write(to: URL(fileURLWithPath: "\(out)/icon_\(n).png"))
}

// Renders synthetic "screenshots" full of FAKE secrets for tests and the demo library.
// usage: swift scripts/make-fixtures.swift <outdir>
import AppKit
import CoreImage

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "fixtures"
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)

struct Page { let name: String; let title: String; let dark: Bool; let mono: Bool; let lines: [String] }
let pages: [Page] = [
  Page(name: "onboarding", title: "Employee record", dark: false, mono: false, lines: [
    "Employee record", "Name  Dana Whitfield", "Department  Facilities", "Social Security number  219-09-9999",
    "Direct deposit", "Routing number  021000021", "Account number  4417 2280 9931", "Start date  March 3, 2026"]),
  Page(name: "terminal", title: "deploy — zsh", dark: true, mono: true, lines: [
    "$ cat .env.staging", "APP_NAME=harbor", "DB_PASSWORD=correct-horse-battery-9", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
    "PORT=8080", "$ git status", "On branch main", "nothing to commit, working tree clean"]),
  Page(name: "notes", title: "Notes", dark: false, mono: false, lines: [
    "House stuff", "Plumber is coming Tuesday at 10", "The wifi is BlueHeron and you get on with",
    "marmalade-tuba-41", "Garage keypad code is 7725", "Pick up paint samples"]),
  Page(name: "checkout", title: "Checkout", dark: false, mono: false, lines: [
    "Checkout", "2 x Field notebook   $24.00", "Card  4111 1111 1111 1111", "Expires 08/29   CVV 737", "Ship to 12 Harbor Lane", "Place order"]),
  Page(name: "clean", title: "Quarterly plan", dark: false, mono: false, lines: [
    "Quarterly plan", "Revenue is up 12% over last quarter", "Order number 88213 shipped on 2026-03-14", "Call Sam at 555-0142 about the launch",
    "Meeting moved to 3:30 PM", "Version 4.2.1 is live", "Password reset page redesign is done"]),
]
func qr(_ s: String) -> CGImage {
  let f = CIFilter(name: "CIQRCodeGenerator")!
  f.setValue(Data(s.utf8), forKey: "inputMessage"); f.setValue("M", forKey: "inputCorrectionLevel")
  let i = f.outputImage!.transformed(by: CGAffineTransform(scaleX: 6, y: 6))
  return CIContext().createCGImage(i, from: i.extent)!
}
let codes = ["otpauth://totp/Acme:dana?secret=JBSWY3DPEHPK3PXP&issuer=Acme", "WIFI:T:WPA;S:BlueHeron;P:marmalade;;", "https://example.com/menu"]
do {
  let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1600, pixelsHigh: 1000, bitsPerSample: 8, samplesPerPixel: 4,
    hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
  NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 1600, height: 1000).fill()
  ("Scan to set up two-step sign-in" as NSString).draw(at: NSPoint(x: 120, y: 850), withAttributes: [.font: NSFont.systemFont(ofSize: 34)])
  for (i, c) in codes.enumerated() { let q = qr(c); NSGraphicsContext.current!.cgContext.draw(q, in: CGRect(x: 120 + i * 460, y: 350, width: q.width, height: q.height)) }
  NSGraphicsContext.restoreGraphicsState()
  try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(out)/qr.png"))
}
for p in pages {
  let W = 1600, H = 1000
  let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: W, pixelsHigh: H, bitsPerSample: 8, samplesPerPixel: 4,
    hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
  (p.dark ? NSColor(white: 0.11, alpha: 1) : NSColor(white: 0.97, alpha: 1)).setFill()
  NSRect(x: 0, y: 0, width: W, height: H).fill()
  (p.dark ? NSColor(white: 0.17, alpha: 1) : NSColor(white: 0.91, alpha: 1)).setFill()
  NSRect(x: 0, y: H - 56, width: W, height: 56).fill()
  let ink = p.dark ? NSColor(white: 0.9, alpha: 1) : NSColor(white: 0.13, alpha: 1)
  (p.title as NSString).draw(at: NSPoint(x: 720, y: H - 40), withAttributes: [.font: NSFont.systemFont(ofSize: 20, weight: .semibold), .foregroundColor: ink])
  var y = H - 150
  for (i, l) in p.lines.enumerated() {
    let f = p.mono ? NSFont.monospacedSystemFont(ofSize: 26, weight: .regular)
                   : NSFont.systemFont(ofSize: i == 0 ? 42 : 28, weight: i == 0 ? .bold : .regular)
    (l as NSString).draw(at: NSPoint(x: 120, y: y), withAttributes: [.font: f, .foregroundColor: ink]); y -= 82
  }
  NSGraphicsContext.restoreGraphicsState()
  try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(out)/\(p.name).png"))
}
print("wrote \(pages.count) fixtures to \(out)")

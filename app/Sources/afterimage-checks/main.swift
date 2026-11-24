import AfterimageKit
import Foundation
let path = CommandLine.arguments[1]
let img = Pixels.load(path)!
for i in 0..<4 { let t = Date(); let l = try Recognizer.lines(in: img); print(i, l.count, Date().timeIntervalSince(t)) }

import AfterimageEngine
import CoreGraphics
import Foundation
import Vision

public enum Recognizer {
    /// Read every line on screen, with a box per word. Throws rather than returning an unread image as "no text".
    public static func lines(in image: CGImage) throws -> [TextLine] {
        let req = VNRecognizeTextRequest()
        req.recognitionLevel = .accurate
        // Correction "fixes" passwords and code into dictionary words.
        req.usesLanguageCorrection = false
        req.automaticallyDetectsLanguage = true
        // A two-step sign-in or Wi-Fi code is a secret no OCR can see; its payload joins the text so the same rules apply.
        let codes = VNDetectBarcodesRequest()
        try autoreleasepool { try VNImageRequestHandler(cgImage: image, options: [:]).perform([req, codes]) }
        guard let found = req.results, let boxes = codes.results else { throw CocoaError(.featureUnsupported) }
        let payloads = boxes.map { code($0.payloadStringValue, $0.boundingBox) }
        return readingOrder(found.compactMap(line)) + payloads
    }

    /// The engine hides every code it can't prove harmless; one that can't be decoded has no text at all.
    static func code(_ payload: String?, _ box: CGRect) -> TextLine {
        TextLine(text: payload ?? "", rect: rect(box), tokens: [], code: true)
    }

    private static func line(_ o: VNRecognizedTextObservation) -> TextLine? {
        guard let c = o.topCandidates(1).first else { return nil }
        let s = c.string
        var tokens: [Token] = []
        var i = s.startIndex
        while i < s.endIndex {
            guard !s[i].isWhitespace else { i = s.index(after: i); continue }
            var j = i
            while j < s.endIndex, !s[j].isWhitespace { j = s.index(after: j) }
            if let box = try? c.boundingBox(for: i..<j) {
                let start = s.unicodeScalars.distance(from: s.startIndex, to: i)
                let len = s.unicodeScalars.distance(from: i, to: j)
                tokens.append(Token(start: UInt32(start), len: UInt32(len), rect: rect(box.boundingBox)))
            }
            i = j
        }
        return TextLine(text: s, rect: rect(o.boundingBox), tokens: tokens)
    }

    /// Vision is bottom-left normalised; the engine is top-left.
    private static func rect(_ r: CGRect) -> Rect {
        Rect(x: r.minX, y: 1 - r.maxY, w: r.width, h: r.height)
    }

    /// Rows top to bottom, then left to right. Near-equal tops share a row so jitter can't shuffle columns.
    public static func readingOrder(_ lines: [TextLine]) -> [TextLine] {
        var rows: [[TextLine]] = []
        for l in lines.sorted(by: { $0.rect.y < $1.rect.y }) {
            if let r = rows.last?.first, l.rect.y - r.rect.y < max(0.006, 0.5 * min(l.rect.h, r.rect.h)) {
                rows[rows.count - 1].append(l)
            } else {
                rows.append([l])
            }
        }
        return rows.flatMap { $0.sorted { $0.rect.x < $1.rect.x } }
    }
}

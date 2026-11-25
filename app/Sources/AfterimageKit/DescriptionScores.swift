import Foundation

/// How good a description is, measured without a judge: what it mentions, whether its specifics are on the screen,
/// and how far two of them agree. Used by the developer comparison of models; nothing here runs in the app.
public enum DescriptionScores {
    /// The share of `facts` the description mentions, ignoring case and extra spaces. No facts: nothing to measure.
    public static func recall(of facts: [String], in description: String) -> Double? {
        let wanted = facts.map(plain).filter { !$0.isEmpty }
        guard !wanted.isEmpty else { return nil }
        let d = plain(description)
        return Double(wanted.filter(d.contains).count) / Double(wanted.count)
    }

    /// The specifics a description commits to: anything with a digit in it, and names (capitalised words that don't
    /// open a sentence).
    public static func specifics(in description: String) -> [String] {
        var out: [String] = [], opening = true
        for raw in description.split(whereSeparator: { $0.isWhitespace }) {
            let word = raw.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
            defer { opening = raw.last.map { ".!?:".contains($0) } ?? false }
            guard word.count > 1, word != "hidden" else { continue }
            if word.contains(where: \.isNumber) || (!opening && word.first!.isUppercase) { out.append(word.lowercased()) }
        }
        return out
    }

    /// The share of a description's specifics that can be found in what was on the screen. Low means it made things up
    /// (or read something the text recogniser didn't). No specifics: nothing to measure.
    public static func grounded(_ description: String, screen: String) -> Double? {
        let s = specifics(in: description)
        guard !s.isEmpty else { return nil }
        let on = plain(screen)
        return Double(s.filter(on.contains).count) / Double(s.count)
    }

    /// How far two descriptions use the same words: shared over all, small words aside.
    public static func overlap(_ a: String, _ b: String) -> Double {
        let (x, y) = (words(a), words(b))
        guard !x.isEmpty || !y.isEmpty else { return 1 }
        return Double(x.intersection(y).count) / Double(x.union(y).count)
    }

    /// How alike two meaning vectors are.
    public static func cosine(_ a: [Double], _ b: [Double]) -> Double {
        guard a.count == b.count, !a.isEmpty else { return 0 }
        let dot = zip(a, b).reduce(0) { $0 + $1.0 * $1.1 }
        let (na, nb) = (a.reduce(0) { $0 + $1 * $1 }.squareRoot(), b.reduce(0) { $0 + $1 * $1 }.squareRoot())
        return na == 0 || nb == 0 ? 0 : dot / (na * nb)
    }

    /// The middle value: one slow picture shouldn't speak for a model.
    public static func median(_ v: [Double]) -> Double? {
        guard !v.isEmpty else { return nil }
        let s = v.sorted()
        return s.count % 2 == 1 ? s[s.count / 2] : (s[s.count / 2 - 1] + s[s.count / 2]) / 2
    }

    static func plain(_ s: String) -> String {
        s.lowercased().split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }
    private static let small: Set<String> = ["the", "a", "an", "of", "and", "to", "in", "is", "are", "for", "on", "with", "at", "by", "it", "its", "this", "that", "as", "from", "or", "be", "was", "has", "have", "there"]
    static func words(_ s: String) -> Set<String> {
        Set(s.lowercased().split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init).filter { $0.count > 1 && !small.contains($0) })
    }
}

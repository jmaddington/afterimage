import Testing
@testable import AfterimageKit

@Suite struct DescriptionScoresTests {
    let screen = "Notes House stuff\nPlumber is coming Tuesday at 10\nThe wifi is BlueHeron\nPick up paint samples"

    @Test func recallCountsTheFactsMentioned() {
        let d = "Notes titled House stuff. A plumber is coming  tuesday at 10."
        #expect(DescriptionScores.recall(of: ["Plumber", "Tuesday", "BlueHeron", "paint"], in: d) == 0.5)
        #expect(DescriptionScores.recall(of: [], in: d) == nil)
    }

    @Test func specificsAreNumbersAndNamesButNotSentenceOpeners() {
        let s = DescriptionScores.specifics(in: "Notes about the house. The plumber comes Tuesday at 10 and the wifi is BlueHeron, [hidden].")
        #expect(s == ["tuesday", "10", "blueheron"])
    }

    @Test func groundedFallsWhenADescriptionInventsThings() {
        #expect(DescriptionScores.grounded("A plumber comes Tuesday at 10.", screen: screen) == 1)
        // Wednesday and 11 are nowhere on the screen.
        #expect(DescriptionScores.grounded("A plumber comes Wednesday at 11, says BlueHeron.", screen: screen) == 1.0 / 3)
        #expect(DescriptionScores.grounded("nothing particular here", screen: screen) == nil)
    }

    @Test func agreementIsSharedWordsAndSharedMeaning() {
        #expect(DescriptionScores.overlap("The plumber is coming", "A plumber coming") == 1)
        #expect(DescriptionScores.overlap("plumber coming tuesday", "paint samples") == 0)
        #expect(DescriptionScores.overlap("plumber tuesday", "plumber wednesday") == 1.0 / 3)
        #expect(abs(DescriptionScores.cosine([1, 0], [1, 0]) - 1) < 1e-9 && DescriptionScores.cosine([1, 0], [0, 1]) == 0 && DescriptionScores.cosine([], []) == 0)
        #expect(DescriptionScores.median([9, 1, 2]) == 2 && DescriptionScores.median([1, 2, 3, 10]) == 2.5 && DescriptionScores.median([]) == nil)
    }
}

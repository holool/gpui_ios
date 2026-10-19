import XCTest

final class NativeSmokeTests: XCTestCase {
    func testPortableTouchAndNativeTextCommit() {
        let app = XCUIApplication()
        app.launchArguments = ["--gpui-ios-accessibility"]
        app.launch()

        let counter = app.buttons["Touch counter"]
        guard counter.waitForExistence(timeout: 15) else {
            XCTFail(app.debugDescription)
            return
        }
        XCTAssertEqual(counter.value as? String, "0")
        counter.tap()
        let incremented = expectation(for: NSPredicate(format: "value == %@", "1"), evaluatedWith: counter)
        wait(for: [incremented], timeout: 5)

        let input = app.descendants(matching: .any)["Native text input"].firstMatch
        XCTAssertTrue(input.exists)
        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        // Stage 1 exposes a semantic input, while UIKit's IME client stays hidden.
        // Tap real software keys instead of requiring an editable AX text field.
        for key in ["a", "b", "c"] { app.keyboards.keys[key].tap() }
        let committed = expectation(for: NSPredicate(format: "value CONTAINS %@", "abc"), evaluatedWith: input)
        wait(for: [committed], timeout: 5)
    }
}

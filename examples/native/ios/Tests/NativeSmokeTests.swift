import XCTest

final class NativeSmokeTests: XCTestCase {
    func testPortableTouchAndNativeTextCommit() {
        let app = XCUIApplication()
        app.launchArguments = ["--gpui-ios-accessibility"]
        app.launch()

        let counter = app.buttons["Touch counter"]
        XCTAssertTrue(counter.waitForExistence(timeout: 15))
        XCTAssertEqual(counter.value as? String, "0")
        counter.tap()
        let incremented = expectation(for: NSPredicate(format: "value == %@", "1"), evaluatedWith: counter)
        wait(for: [incremented], timeout: 5)

        let input = app.descendants(matching: .any)["Native text input"].firstMatch
        XCTAssertTrue(input.exists)
        input.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 10))
        app.typeText("abc")
        let committed = expectation(for: NSPredicate(format: "value CONTAINS %@", "abc"), evaluatedWith: input)
        wait(for: [committed], timeout: 5)
    }
}

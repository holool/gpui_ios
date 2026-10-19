# gpui_ios

Native UIKit backend for [Holool GPUI Fast](https://github.com/holool/gpui-fast). GPUI renders directly into a `UIView` backed by `CAMetalLayer`, using the shared `gpui_apple::MetalRenderer` and `AppleTextSystem`. It retains Fast's bidi shaping and scroll tile renderer; no older mobile renderer or text shaper is copied here.

The reusable Swift host owns UIKit's run loop, `CADisplayLink`, lifecycle, keyboard geometry and `UITextInput`. Application views and state remain Rust.

## Build the native example

Requires macOS, Xcode with the iOS SDK and Metal Toolchain, Rust 1.95, and XcodeGen.

```sh
brew install xcodegen
xcodebuild -downloadComponent MetalToolchain
bash scripts/build-example.sh
```

The script builds a Rust static library and a complete simulator app at `examples/native/ios/build/Build/Products/Debug-iphonesimulator/Native.app`. It does not require a signing identity. For a signed device distribution, use your own Xcode signing settings.

```sh
xcrun simctl boot 'iPhone 16 Pro' # skip if already booted
xcrun simctl install booted examples/native/ios/build/Build/Products/Debug-iphonesimulator/Native.app
xcrun simctl launch booted dev.holool.gpui-ios-example
```

`GPUI_RUNTIME_SHADERS=1` enables the shared renderer's runtime shader compilation path for development without the separate Metal compiler component. `GPUI_BUILD_PROFILE=release` builds optimized Rust. `GPUI_IOS_TARGET=aarch64-apple-ios` selects device compilation; the example script still disables signing.

For coordinated development, `GPUI_FAST_PATH=/absolute/path/to/gpui-fast` supplies Cargo path overrides for both shared packages. Published dependencies use one exact Fast Git revision. Do not mix an independent GPUI framework into the same app.

## Embed in an application

Compile `ios/GPUIHost.swift` into a UIKit application. Include `ios/gpui_ios.h` from its Swift bridging header, together with your Rust entrypoint declaration:

```c
#include "gpui_ios.h"
void my_app_launch(void);
```

Subclass `GPUIAppDelegate` in the app's `@main` delegate and override `start()` to call `my_app_launch()`. The Rust static library entrypoint can share its view across desktop and mobile:

```rust,ignore
#[unsafe(no_mangle)]
pub extern "C" fn my_app_launch() {
    gpui_ios::run(|cx| {
        cx.open_window(gpui::WindowOptions::default(), |_, cx| {
            cx.new(MyApplication::new)
        }).expect("open the application window");
    });
}
```

`run` retains `ApplicationHandle` while UIKit drives events. For custom asset or HTTP configuration, use `run_with(application().with_assets(MyAssets), launch)`. Both helpers retain the configured app until `shutdown()` releases it when the native host is actually terminating. Returning from ordinary `Application::run` does not retain an embedded app.

`data_directory()` returns the sandbox's Application Support directory without modifying process environment variables. Clipboard and Keychain use the native Apple services. Unsupported document pickers and auxiliary executables return explicit errors.

## Input and accessibility

Touch contacts keep stable monotonically increasing IDs through end or cancellation and enter Fast as raw `TouchEvent`s. The shared portable gesture arena owns tap, pan and long press recognition. Backgrounding cancels outstanding touches and pauses frame delivery.

The Swift `UITextInput` client forwards UTF-16 selection, marked composition and commits to the focused GPUI handler. Input components must implement `text_length_utf16`, selection mutation and shaped range geometry. Backspace removes a complete grapheme, including Arabic combining marks and emoji ZWJ sequences. Directional native queries use actual GPUI caret geometry; they do not treat a decreasing logical index as movement to the left. The native writing direction is natural. Keyboard frame changes continuously publish insets during transitions and are exact at rest; floating keyboards do not become a full-width bottom inset. Applications can use `Window::fully_visible_bounds()` for avoidance.

Stage 1 retains GPUI Kit's selection painting and handles; it does not install UIKit selection handles or promise native paragraph-direction editing. UIKit IME range queries use the focused component's actual geometry. Hardware navigation keys enter GPUI's normal keymap; candidate navigation and confirmation stay with UIKit while marked composition is active.

AccessKit updates are mirrored to stable `UIAccessibilityElement`s in source reading order. The bridge supplies labels, values, logical bounds, basic role/disabled traits, and click/focus actions. It prunes removed nodes. This is a semantic baseline, not a complete AccessKit iOS adapter: text rotors, editable accessibility actions, live-region announcements and advanced table navigation remain unimplemented.

## Validation

```sh
cargo test -p gpui_ios --lib
cargo check --target aarch64-apple-ios-sim --workspace
bash scripts/build-example.sh
```

Run `bash scripts/test-example.sh SIMULATOR_UDID` for the native UI smoke test. It taps the portable touch counter, focuses the GPUI input, opens UIKit's keyboard and checks a native text commit through the semantic tree. The `--gpui-ios-accessibility` app launch argument enables that tree for UI automation without changing the simulator's VoiceOver settings.

The example includes Arabic/Latin/emoji text input, a portable touch counter and semantic roles. Validate Arabic selection, CJK composition, candidate geometry, paste, rotation, keyboard avoidance, foreground/background and VoiceOver on real devices before making production claims. Simulator execution does not establish device GPU compatibility, battery consumption or performance.

## License

Apache-2.0. GPUI Fast and its shared Apple backend are upstream dependencies with their own notices. [Zedra's mobile GPUI work](https://github.com/tanlethanh/zed/tree/feat/gpui-mobile) informed the platform audit; its old renderer, shaper and native-selection protocol are not vendored.

#!/usr/bin/env bash
set -euo pipefail
task_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$task_root"
target="${GPUI_IOS_TARGET:-aarch64-apple-ios-sim}"
profile="${GPUI_BUILD_PROFILE:-debug}"
args=(--target "$target" -p gpui-ios-native-example)
if [[ "$profile" == release ]]; then args+=(--release); fi
if [[ "${GPUI_RUNTIME_SHADERS:-0}" == 1 ]]; then args+=(--features gpui_ios/runtime_shaders); fi
if [[ -n "${GPUI_FAST_PATH:-}" ]]; then
  args+=(--config "patch.\"https://github.com/holool/gpui-fast\".gpui.path=\"$GPUI_FAST_PATH/crates/gpui\"")
  args+=(--config "patch.\"https://github.com/holool/gpui-fast\".gpui_apple.path=\"$GPUI_FAST_PATH/crates/gpui_apple\"")
else
  args+=(--locked)
fi
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cargo "+${GPUI_RUST_TOOLCHAIN:-1.99.0}" build "${args[@]}"
xcodegen generate --spec examples/native/ios/project.yml
rust_target="${CARGO_TARGET_DIR:-$task_root/target}"
if [[ "$rust_target" != /* ]]; then rust_target="$task_root/$rust_target"; fi
sdk=iphonesimulator
destination='generic/platform=iOS Simulator'
if [[ "$target" == aarch64-apple-ios ]]; then sdk=iphoneos; destination='generic/platform=iOS'; fi
xcodebuild -project examples/native/ios/Native.xcodeproj -scheme Native -configuration Debug -sdk "$sdk" -destination "$destination" -derivedDataPath examples/native/ios/build GPUI_RUST_LIBRARY_DIR="$rust_target/$target/$profile" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO build

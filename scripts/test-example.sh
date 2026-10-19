#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 1 ]]; then
  echo "Usage: $0 SIMULATOR_UDID" >&2
  exit 2
fi
task_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$task_root"+bash scripts/build-example.sh
rust_target="${CARGO_TARGET_DIR:-$task_root/target}"
if [[ "$rust_target" != /* ]]; then rust_target="$task_root/$rust_target"; fi
xcodebuild -project examples/native/ios/Native.xcodeproj -scheme Native -configuration Debug -sdk iphonesimulator -destination "id=$1" -derivedDataPath examples/native/ios/build GPUI_RUST_LIBRARY_DIR="$rust_target/aarch64-apple-ios-sim/${GPUI_BUILD_PROFILE:-debug}" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO test

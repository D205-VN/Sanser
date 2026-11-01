#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
rust_target="${1:?Expected a Rust iOS/Android target triple}"
export CARGO_TARGET_DIR="${SANSER_MOBILE_RUST_TARGET_DIR:-$repo_dir/apps/mobile/build/rust-p2p}"
case "$rust_target" in
  aarch64-apple-ios) export SDKROOT="$(xcrun --sdk iphoneos --show-sdk-path)" ;;
  aarch64-apple-ios-sim|x86_64-apple-ios) export SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)" ;;
  aarch64-linux-android)
    ndk="${SANSER_ANDROID_NDK:-${ANDROID_NDK_HOME:-}}"
    if [[ -z "$ndk" ]]; then ndk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}/ndk/28.2.13676358"; fi
    ndk_host=linux-x86_64
    if [[ "$(uname -s)" == Darwin ]]; then ndk_host=darwin-x86_64; fi
    compiler="$ndk/toolchains/llvm/prebuilt/$ndk_host/bin/aarch64-linux-android26-clang"
    [[ -x "$compiler" ]] || { echo 'Android NDK 28.2.13676358 is required for mobile P2P.' >&2; exit 2; }
    export CC_aarch64_linux_android="$compiler"
    export AR_aarch64_linux_android="$(dirname "$compiler")/llvm-ar"
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$compiler"
    ;;
  *) echo 'Unsupported mobile P2P target' >&2; exit 2 ;;
esac
cd "$repo_dir"
cargo build --locked -p sanser-mobile-p2p --release --target "$rust_target"

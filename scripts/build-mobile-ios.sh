#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
ios_sdk="${1:-iphoneos}"
case "$ios_sdk" in iphoneos|iphonesimulator) ;; *) echo "Expected iphoneos or iphonesimulator" >&2;exit 2;; esac
# Flutter's simulator app contains both slices even on an Apple Silicon runner.
# Build each independently so Opus configures the correct SIMD implementation.
ios_archs=(arm64)
if [[ "$ios_sdk" == iphonesimulator ]]; then ios_archs+=(x86_64); fi
archives=()
for ios_arch in "${ios_archs[@]}"; do
  build_dir="$repo_dir/apps/mobile/build/native-$ios_sdk-$ios_arch"
  cmake -S "$repo_dir/native/mobile" -B "$build_dir" \
    -DCMAKE_SYSTEM_NAME=iOS -DCMAKE_SYSTEM_PROCESSOR="$ios_arch" \
    -DCMAKE_OSX_SYSROOT="$ios_sdk" -DCMAKE_OSX_ARCHITECTURES="$ios_arch" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=15.0 -DCMAKE_BUILD_TYPE=Release
  cmake --build "$build_dir" --parallel 4
  archive="$build_dir/libSanserMobile.a"
  xcrun libtool -static -o "$archive" \
    "$build_dir/libsanser_mobile.a" "$build_dir/libsanser-audio-codec.a" \
    "$build_dir/protocol/libsanser-snv2.a" "$build_dir/_deps/sanser_opus-build/libopus.a"
  archives+=("$archive")
done
mkdir -p "$repo_dir/apps/mobile/ios/Native/$ios_sdk"
xcrun lipo -create "${archives[@]}" -output "$repo_dir/apps/mobile/ios/Native/$ios_sdk/libSanserMobile.a"
xcrun lipo -info "$repo_dir/apps/mobile/ios/Native/$ios_sdk/libSanserMobile.a"

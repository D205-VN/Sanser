#!/usr/bin/env bash
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
build_dir="$(mktemp -d "${TMPDIR:-/tmp}/sanser-video-test.XXXXXX")"
trap 'rm -rf "$build_dir"' EXIT
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
xcrun --sdk macosx swiftc -swift-version 5 \
  "$repo_dir/apps/mobile/ios/Runner/MobileVideoDecoder.swift" \
  "$repo_dir/native/mobile/video_decoder_test.swift" \
  -framework VideoToolbox -framework CoreMedia -framework CoreVideo \
  -o "$build_dir/video-test"
"$build_dir/video-test"

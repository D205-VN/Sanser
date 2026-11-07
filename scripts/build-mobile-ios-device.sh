#!/usr/bin/env bash
# Build an unsigned, AOT iPhone IPA; recipient signing remains a separate step.
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
flutter_bin="${FLUTTER_BIN:-$(command -v flutter || true)}"
if [[ -z "$flutter_bin" ]]; then echo 'Set FLUTTER_BIN to Flutter 3.47.7.' >&2; exit 2; fi
export DEVELOPER_DIR="${DEVELOPER_DIR:-$(xcode-select -p)}"
if [[ ! -d "$DEVELOPER_DIR/Platforms/iPhoneOS.platform" ]]; then
  echo 'Set DEVELOPER_DIR to the full Xcode.app/Contents/Developer directory.' >&2
  exit 2
fi
output_dir="${1:-$repo_dir/artifacts/mobile-2.3.5}"
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"
ipa="$output_dir/Sanser-iOS-2.3.5-device-unsigned.ipa"
if [[ -e "$ipa" ]]; then echo "Already exists: $ipa" >&2; exit 2; fi
bash "$repo_dir/scripts/build-mobile-ios.sh" iphoneos

# Keep release signing/AOT intermediates away from cloud-synced Documents folders,
# where File Provider can attach FinderInfo while Xcode is copying frameworks.
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/sanser-iphone-build.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
mkdir -p "$work_dir/apps/mobile" "$work_dir/native/mobile" "$work_dir/tools"
rsync -rlt --exclude=/android --exclude=/build --exclude=/.dart_tool \
  --exclude=/ios/Pods --exclude=/ios/.symlinks --exclude=/ios/Native \
  --exclude=/ios/Flutter/ephemeral --exclude=/ios/Flutter/Generated.xcconfig \
  --exclude=/ios/Flutter/flutter_export_environment.sh \
  "$repo_dir/apps/mobile/" "$work_dir/apps/mobile/"
mkdir -p "$work_dir/apps/mobile/ios/Native/iphoneos"
cp "$repo_dir/apps/mobile/ios/Native/iphoneos/libSanserMobile.a" \
  "$work_dir/apps/mobile/ios/Native/iphoneos/"
cp "$repo_dir/native/mobile/client.h" "$work_dir/native/mobile/"
cp "$repo_dir/native/mobile/p2p.h" "$work_dir/native/mobile/"

# hooks_runner intentionally strips DEVELOPER_DIR from its environment, but keeps
# PATH. A scoped wrapper selects the same Xcode without changing xcode-select.
printf '#!/bin/bash\nexport DEVELOPER_DIR=%q\nexec /usr/bin/xcrun "$@"\n' \
  "$DEVELOPER_DIR" > "$work_dir/tools/xcrun"
chmod +x "$work_dir/tools/xcrun"
export PATH="$work_dir/tools:$PATH"
cd "$work_dir/apps/mobile"
"$flutter_bin" pub get --enforce-lockfile
if ! "$flutter_bin" build ios --release --no-codesign \
  > "$output_dir/ios-device-build.log" 2>&1; then
  tail -n 100 "$output_dir/ios-device-build.log"
  exit 1
fi
python3 "$repo_dir/scripts/package-mobile-ios.py" build/ios/iphoneos/Runner.app "$ipa"
echo "Requires local re-signing before installation: $ipa"

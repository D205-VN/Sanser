#!/bin/bash
# Keep Xcode's selected SDK visible inside Flutter's isolated native build hooks.
set -euo pipefail
: "${FLUTTER_ROOT:?Flutter/Generated.xcconfig must define FLUTTER_ROOT}"
: "${DERIVED_FILE_DIR:?Run this script from the Runner Xcode scheme/build phase}"
export DEVELOPER_DIR="${DEVELOPER_DIR:-$(/usr/bin/xcode-select -p)}"
if [[ ! -d "$DEVELOPER_DIR/Platforms/iPhoneOS.platform" ]]; then
  echo 'error: Sanser requires full Xcode, including the iPhoneOS SDK.' >&2
  exit 1
fi

# hooks_runner passes PATH to hooks, but removes DEVELOPER_DIR. Generate a local
# shim with the current build's Xcode path, rather than changing xcode-select.
# Xcode can run the scheme's prepare action and a target phase independently.
tools_dir="$DERIVED_FILE_DIR/sanser-xcode-tools"
mkdir -p "$tools_dir"
shim="$(mktemp "$tools_dir/xcrun.XXXXXX")"
printf '#!/bin/bash\nexport DEVELOPER_DIR=%q\nexec /usr/bin/xcrun "$@"\n' \
  "$DEVELOPER_DIR" > "$shim"
chmod +x "$shim"
mv -f "$shim" "$tools_dir/xcrun"
export PATH="$tools_dir:$PATH"
exec /bin/sh "$FLUTTER_ROOT/packages/flutter_tools/bin/xcode_backend.sh" "$@"

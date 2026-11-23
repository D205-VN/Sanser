#!/usr/bin/env bash
# Bumps only the part that changed:
#   scripts/bump-version.sh desktop 2.3.32   # Mac/Windows apps, tag v2.3.32
#   scripts/bump-version.sh mobile  2.3.32   # iPhone/Android apps, tag mobile-v2.3.32
# When both changed, run it for each. Release notes stay a manual step:
# docs/releases/vX.md (desktop) and docs/releases/mobile-vX.md (phone).
set -euo pipefail
cd "$(dirname "$0")/.."
part="${1:-}" next="${2:-}"
[[ "$part" == desktop || "$part" == mobile ]] && [[ "$next" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "usage: $0 desktop|mobile X.Y.Z" >&2; exit 2; }
replace() { # file old new
  local file="$1" old="${2//./\\.}" new="$3"
  sed -i.bak "s/${old}/${new}/g" "$file" && rm -f "$file.bak"
}
if [[ "$part" == desktop ]]; then
  current="$(sed -n 's/^readonly expected_version="\(.*\)"/\1/p' scripts/check-repository.sh)"
  for file in .env.example .github/workflows/release.yml Cargo.toml apps/desktop/index.html \
      apps/desktop/package.json apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json \
      apps/desktop/src/lib/types.ts crates/sanser-core/src/lib.rs crates/sanser-diagnostics/src/export.rs \
      docs/release.md native/client-macos/CMakeLists.txt native/common/sanser_version.h \
      native/desktop/CMakeLists.txt native/host-windows/CMakeLists.txt package-lock.json package.json \
      scripts/check-repository.sh website/index.html; do
    replace "$file" "$current" "$next"
  done
  sed -i.bak "1s/^# Sanser .*/# Sanser ${next}/; s/^Product:  Sanser .*/Product:  Sanser ${next}/" README.md && rm -f README.md.bak
  cargo update -w --offline >/dev/null
  echo "desktop: $current -> $next (tag v$next)"
else
  line="$(grep -E '^version: [0-9.]+\+[0-9]+$' apps/mobile/pubspec.yaml)"
  current="${line#version: }"; current="${current%%+*}"; build="${line##*+}"
  sed -i.bak "s/^version: .*/version: ${next}+$((build + 1))/" apps/mobile/pubspec.yaml && rm -f apps/mobile/pubspec.yaml.bak
  replace apps/mobile/lib/src/api.dart "const mobileVersion = '$current';" "const mobileVersion = '$next';"
  replace native/mobile/CMakeLists.txt "VERSION $current " "VERSION $next "
  echo "mobile: $current -> $next, build $((build + 1)) (tag mobile-v$next)"
fi

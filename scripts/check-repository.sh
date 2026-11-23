#!/usr/bin/env bash
set -euo pipefail

readonly expected_version="2.3.31"
readonly expected_app_id="com.sanser.desktop"

fail() {
  printf 'repository policy: %s\n' "$1" >&2
  exit 1
}

[[ -f Cargo.toml ]] || fail "Cargo.toml is missing"
[[ -f package.json ]] || fail "package.json is missing"
[[ -f apps/desktop/src-tauri/tauri.conf.json ]] || fail "Tauri configuration is missing"

grep -Fq "version = \"${expected_version}\"" Cargo.toml || fail "Cargo workspace version is not ${expected_version}"
grep -Fq "\"version\": \"${expected_version}\"" package.json || fail "root package version is not ${expected_version}"
grep -Fq "\"identifier\": \"${expected_app_id}\"" apps/desktop/src-tauri/tauri.conf.json || fail "Tauri identifier is not ${expected_app_id}"
grep -Fq "\"version\": \"${expected_version}\"" apps/desktop/package.json || fail "desktop package version is not ${expected_version}"
grep -Fq "version = \"${expected_version}\"" apps/desktop/src-tauri/Cargo.toml || fail "desktop Rust version is not ${expected_version}"
grep -Fq "\"version\": \"${expected_version}\"" apps/desktop/src-tauri/tauri.conf.json || fail "Tauri bundle version is not ${expected_version}"
grep -Fq "SANSER_VERSION=${expected_version}" .env.example || fail ".env.example version is not ${expected_version}"
grep -Fq "SANSER_VERSION \"${expected_version}\"" native/common/sanser_version.h || fail "native fallback version is not ${expected_version}"
grep -Fq "VERSION ${expected_version}" native/client-macos/CMakeLists.txt || fail "macOS native project version is not ${expected_version}"
grep -Fq "VERSION ${expected_version}" native/host-windows/CMakeLists.txt || fail "Windows native project version is not ${expected_version}"
grep -Fq "SANSER_VERSION = '${expected_version}'" apps/desktop/src/lib/types.ts || fail "frontend version is not ${expected_version}"

for retired in desktop/main.js desktop/preload.js server.js scripts/install-tailscale.js; do
  [[ ! -e "$retired" ]] || fail "retired runtime file still exists: ${retired}"
done

if git ls-files | grep -E '(^|/)(node_modules|dist|build|native-captures)/' >/dev/null; then
  fail "generated build output is tracked"
fi

if git ls-files | grep -E '(^|/)\.env($|\.)' | grep -vE '(^|/)\.env\.example$' >/dev/null; then
  fail "an environment secret file is tracked"
fi

if git grep -IEn 'GameRemote|gameremote-parsec-like|TAILSCALE_USE_STUN|tailscale[.]com' -- \
  ':!work.txt' ':!scripts/check-repository.sh' 2>/dev/null; then
  fail "retired branding or Tailscale-specific configuration remains"
fi

if git grep -IEn 'postgres(ql)?://[^[:space:]]+@[^[:space:]]*neon[.]tech' -- \
  ':!work.txt' ':!scripts/check-repository.sh' \
  | grep -Ev 'postgres(ql)?://(USER:PASSWORD|user(:((pass)|(secret)))?)@' >/dev/null; then
  fail "a non-placeholder Neon connection string is tracked"
fi

# The phone app is versioned separately: it changes only when the phone app
# changes. Its version lives in pubspec.yaml and must match the app constant
# and the native mobile library.
mobile_version="$(sed -n 's/^version: \([0-9][0-9.]*\)+[0-9][0-9]*$/\1/p' apps/mobile/pubspec.yaml)"
[[ -n "$mobile_version" ]] || fail "apps/mobile/pubspec.yaml needs version: X.Y.Z+BUILD"
grep -Fq "const mobileVersion = '${mobile_version}';" apps/mobile/lib/src/api.dart || fail "mobileVersion is not ${mobile_version}"
grep -Fq "project(sanser_mobile VERSION ${mobile_version} " native/mobile/CMakeLists.txt || fail "native mobile project version is not ${mobile_version}"
[[ -f "docs/releases/mobile-v${mobile_version}.md" || -f "docs/releases/v${mobile_version}.md" ]] || fail "missing release notes for mobile ${mobile_version}"

printf 'repository policy: ok (desktop %s, mobile %s, protocol v2)\n' "$expected_version" "$mobile_version"

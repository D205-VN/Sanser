#!/usr/bin/env bash
set -euo pipefail

readonly expected_version="2.0.2"
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

printf 'repository policy: ok (%s, protocol v2)\n' "$expected_version"

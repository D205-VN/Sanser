# Release

The current release definition is [Sanser 2.3.33](releases/v2.3.33.md), tagged
`v2.3.33`. Its workflow publishes Mac/Windows installers and signed updater
bundles. Enhanced voice is an off-by-default experiment and needs a configured
SFU/gateway; see the release notes for validation and known limitations.
See [sharing implementation and validation progress](multi-viewer-progress.md)
for the remaining real-device checks.

## Desktop and phone versions

The desktop apps (Mac, Windows) and the phone apps (iPhone, Android) have
separate version numbers. Only the part that changed gets a new version;
when both change, both do.

| Part | Version lives in | Bump | Tag | Workflow |
| --- | --- | --- | --- | --- |
| Desktop | `scripts/check-repository.sh` and the desktop/native files | `scripts/bump-version.sh desktop X.Y.Z` | `vX.Y.Z` | `release.yml` |
| Phone | `apps/mobile/pubspec.yaml` (plus `mobileVersion`, `native/mobile`) | `scripts/bump-version.sh mobile X.Y.Z` | `mobile-vX.Y.Z` | `mobile-release.yml` |

A desktop-only change bumps desktop only; a phone-only change bumps the phone
only. Code shared by both (`native/desktop/transport.cpp`, `native/protocol`,
`native/common`) changes both. Connections do not depend on the versions
matching: both sides speak protocol v2.

Phone releases are separate GitHub releases named "Sanser Mobile X.Y.Z" and
are never marked latest, so the desktop updater is unaffected. They carry the
unsigned iPhone IPA and, when the Android release key secrets
(`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`,
`ANDROID_KEY_PASSWORD`) are set, the signed Android APK. The website picks
the newest release that has each file.

The [2.1.22 Preview notes](releases/v2.1.22.md) and
[Sanser 2.0.0 release record](sanser-2-release.md) are historical.

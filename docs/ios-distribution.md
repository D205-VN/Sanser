# Sanser on physical iPhones

Verified on 2026-11-01. Sanser Mobile 2.3.1 is a Flutter preview client. Its current
transport is WSS Relay; Direct/UDP Relay, mobile hosting and microphone/group
voice are not implemented. A successful build does not establish connection,
input, playback or latency correctness on a physical iPhone.

## Distribution choices

| Method | Publisher membership | Recipient experience | Appropriate use |
| --- | --- | --- | --- |
| TestFlight | Apple Developer Program | Install TestFlight, open invitation/public link | Broad beta; up to 10,000 external testers, review required, each build lasts up to 90 days |
| App Store | Apple Developer Program | Normal App Store install and updates | General public release after App Review |
| IPA + AltStore Classic / SideStore | No paid publisher team needed to produce unsigned bytes | Each user signs with their own Apple Account; free provisioning normally needs renewal every 7 days and allows 3 active sideloaded apps including the installer | Technical testers willing to manage signing; not a one-tap public installer |
| Xcode Personal Team | Free Apple Account | Connect own iPhone to own Mac, select own team and run | Personal development/testing, not distribution of one shared signed IPA to everyone |
| Ad Hoc / Enterprise | Paid program and its eligibility requirements | Device registration or organizational deployment | Ad Hoc is device-limited; Enterprise is for internal employee use, not public distribution |
| Alternative marketplace / native Web Distribution | Apple authorization, membership, notarization and regional eligibility | Eligible marketplace/website installation | Region-specific, not a general distribution route for users in Vietnam |
| Browser/PWA | No iOS signing | Open HTTPS site, optionally add to Home Screen | Requires a new browser media implementation; current C++ SNV2/PlatformView client cannot simply become Flutter Web |

Apple Developer Program is normally USD 99/year (local pricing/tax may vary).
Only the publisher needs membership for TestFlight/App Store; testers do not buy
a developer membership. Eligible nonprofit organizations, accredited educational
institutions and government entities can apply for a fee waiver. An individual's
student status alone is not that institutional exemption. A legitimate publishing
partner can publish under its own team with an agreement and responsibility for
the app; that does not eliminate Apple membership or review requirements.

Sources: [Apple membership](https://developer.apple.com/programs/whats-included/),
[TestFlight](https://developer.apple.com/testflight/),
[TestFlight build validity](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/),
[free account limitations](https://developer.apple.com/help/account/basics/about-your-developer-account/),
[fee waivers](https://developer.apple.com/help/account/membership/fee-waivers),
[Enterprise scope](https://developer.apple.com/help/account/membership/programs-overview),
[EU native Web Distribution](https://developer.apple.com/support/web-distribution-eu/),
[AltStore Classic](https://faq.altstore.io/altstore-classic/your-altstore),
[SideStore FAQ](https://docs.sidestore.io/docs/faq).

## Install the unsigned device preview with your own account

1. Obtain **`Sanser-iOS-2.3.1-device-unsigned.ipa`**, plus its checksum. The existing
   `Sanser-iOS-2.3.1-simulator-unsigned.zip` is only for the Mac's Simulator. Renaming
   it to `.ipa` does not turn its binaries into iPhone binaries.
2. Install AltStore **Classic** using the official
   [Mac](https://faq.altstore.io/altstore-classic/how-to-install-altstore-macos) or
   [Windows](https://faq.altstore.io/altstore-classic/how-to-install-altstore-windows)
   guide. Use your own Apple Account locally. Check that the current installer
   supports your iOS version. No Apple credentials belong in Sanser or a chat.
3. Import Sanser's IPA through AltStore's My Apps/add-IPA action. AltServer must be
   reachable as required by its guide. Enable Developer Mode on iOS if requested
   by the system and trust your own developer profile in Settings if necessary.
4. Refresh before the free profile expires. AltStore's background refresh is
   conditional on its operating requirements; it is not an indefinite signature.
   SideStore is another option using its official
   [installation instructions](https://docs.sidestore.io/docs/installation/install),
   with its own setup and refresh requirements.
5. Open Sanser, use the same Sanser server/account as the desktop, and request a
   connection to an online Windows/macOS host. Approve it at the host. Test video,
   touch/ASCII input, system audio and disconnect before distributing more widely.

Sanser is built in release/AOT mode; this preview does not require enabling JIT.
Do not distribute a single free Personal Team profile to many people's devices.
Do not use leaked/shared enterprise certificates or ask users to disable iOS
security protections. Re-signing may change application identity, so retain the
same account/tool when updating to preserve access to stored credentials.

## Build the re-signable IPA

From the repository root with pinned Flutter 3.47.7, full Xcode and CocoaPods:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
FLUTTER_BIN=/absolute/path/to/flutter/bin/flutter \
bash scripts/build-mobile-ios-device.sh
```

The script builds the shared native ARM64 library, stages the iOS application
outside cloud-synced folders, and uses `flutter build ios --release --no-codesign`.
Its temporary xcrun wrapper preserves the selected Xcode for native build hooks
that filter out `DEVELOPER_DIR`. It does not change `xcode-select` or require an
Apple login. Signing assets are not copied into staging.

`scripts/package-mobile-ios.py APP OUTPUT.ipa` verifies the `iPhoneOS` plist,
physical-iOS ARM64 Mach-O platform commands for all executable images, unencrypted
binaries, Flutter AOT presence, no personal provisioning profile, ZIP integrity
and checksum. The output is `Payload/Sanser.app/...`, plus `.ipa.sha256` and
`.ipa.json` reports. Existing output is never overwritten. Re-signing will change
the IPA checksum; compare the published checksum before re-signing.

To run from Xcode instead, build the native library with
`bash scripts/build-mobile-ios.sh iphoneos`, open
`apps/mobile/ios/Runner.xcworkspace`, select Runner, add your Apple Account in
Xcode, select your local Personal Team, connect/trust the iPhone and Run. Keep
team/profile changes local; do not commit personal signing configuration.

## REA findings

The official [REA skill](https://github.com/morluto/rea/blob/main/.agents/skills/reverse-engineer-anything/SKILL.md)
was used via REA CLI 6.8.0 under isolated Node 24.22.1. No disassembler or target
execution was required. Metadata/signature inspection does not demonstrate runtime
performance or compatibility with every signing tool/iOS release.

- Published Simulator bundle: REA `inspect-plist` evidence
  `ev_c5e30709e4672d07b491283ad346edb74f66aefa6110b97bd2520ecf7a93a3f4`
  observed `com.sanser.mobile`, 2.3.1/build 20301,
  `CFBundleSupportedPlatforms=[iPhoneSimulator]`, minimum OS 15.0.
- Physical iPhone build uses the same Flutter/native application source, compiled
  separately for `iphoneos` in release mode. REA `inspect-plist` evidence
  `ev_dfad4ce43628b2f6e862845e30ab4a88701232d0c466c9d0c38cfeb0ce1233dd`
  observed `iPhoneOS`, `iphoneos`, version 2.3.1 and minimum OS 15.0.
  REA `inspect-signature` evidence
  `ev_9035e00fdd8aa0e1a6b74b7d770d011c67cdb8636bc39ae52623e817b193e5ba`
  found no signing team or signature on Runner. Its signature is deliberately
  absent pending recipient provisioning. It cannot be installed merely by opening
  an unsigned download link. Evidence records remain local in the temporary
  `sanser-ios-research-2026-11-01` research directory.
- RustDesk's official 1.5.0 release links iOS users to the App Store rather than
  publishing a universal unsigned IPA installer. This source observation is
  separate from REA's inspection of Sanser's shipped bundle:
  [RustDesk release](https://github.com/rustdesk/rustdesk/releases/tag/1.5.0).

Remaining verification: real iPhone installation with a recipient account,
four-way desktop interoperability as applicable, audio routes, reconnect,
background cleanup, and current AltStore/SideStore compatibility. Publishing via
TestFlight/App Store also needs the publisher's Apple team, app record, privacy
information and review; none of those were created by the unsigned build.

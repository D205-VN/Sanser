# Sanser Mobile — Flutter 2.3.2 preview

Flutter/Dart provides the Android and iOS interface. A native C++ adapter reuses
Sanser's authenticated SNV2 transport, input delivery, frame reassembly and Opus
decoder. Compressed video and PCM never pass through Dart method channels.

Implemented in source:

- HTTPS account login, refresh tokens in Keychain/Android encrypted storage,
  stable installation/device identity and computer discovery.
- Request remote control or view-only access to a Windows/macOS host; approve or
  decline group invitations. View-only input is disabled by the native Peer.
- Direct UDP using the desktop Rust P2P core (authenticated SNP2, STUN gathering,
  measured nomination, reserved socket handoff, standby monitoring and FEC), or
  native authenticated WSS Relay with the desktop SNR1 AEAD envelope.
- H.264 native rendering and 48 kHz system-audio playback. iOS pulls continuous
  stereo PCM from a bounded, lock-free ring instead of scheduling 5 ms buffers;
  underrun/backlog transitions crossfade, and hardware route changes rebuild the
  audio graph. The realtime audio callback does not allocate or lock.
- Absolute touch/drag, HID buttons and an ASCII keyboard; disconnect/release input
  on exit/background. Session state changes only after the native engine reports
  its first frame; RTT comes from actual authenticated probes.
- Draggable session controls: tap the compact button to open actions, collapse it
  or hide it into an edge tab, then tap the arrow to recover it. Controls remain
  inside the safe viewport after rotation and when the keyboard is visible.
- A Settings tab and in-session settings sheet persist automatic landscape,
  Auto/Direct/Relay selection, video bitrate ceiling, initial system audio, compact controls and RTT visibility on this phone.
  Sessions request landscape by default and restore portrait on exit; platform
  restrictions on large Android screens/iPad multitasking can override rotation.

Initial preview limits:

- Auto tries verified IPv4 Direct first, then falls back to WSS Relay. Forced
  Direct reports failure instead of silently switching transports. UDP Relay,
  native IPv6 media and mobile router port mapping are not implemented yet.
  School/guest Wi-Fi isolation and some NAT configurations can prevent Direct.
  The floating panel shows the selected transport and measured native RTT.
- Group sessions honor the host's network mode; the phone's preference applies
  when requesting a new session. No fixed Internet latency promise is made.
- **Group voice and microphone capture are not enabled on mobile.** System audio
  is separate and supported in source. The UI hides unavailable voice actions.
- No mobile host, gamepad, full IME/Unicode text injection or background control.
- Requires a Sanser server accepting **2.3.x clients**; desktop
  Windows/macOS hosts must expose the bidirectional SNV2 engine.
- Builds/unit tests do not replace real Android/iPhone-to-host testing.

## Build

Pinned Flutter: **3.47.7**, revision
`abaf9c523780a608bd46686fd5e53740a07077f8`. Android: JDK 17, SDK 36, NDK
28.2.13676358, CMake 3.22.1. Rust **1.97.1** with `aarch64-linux-android`
for Android, `aarch64-apple-ios` for iPhone, and `aarch64-apple-ios-sim` /
`x86_64-apple-ios` for Simulator. Android ARM64 / Android 8+ / iOS 15+.
The native build scripts compile and link `sanser-mobile-p2p` automatically.
Set `ANDROID_HOME` to the Android SDK and ensure its pinned NDK is installed.

```sh
cd apps/mobile
flutter pub get --enforce-lockfile
flutter analyze
flutter test
flutter build apk --debug --target-platform android-arm64
cd android
./gradlew :app:testDebugUnitTest
```

The Android debug APK is for testing. Production signing must use a persistent
private keystore, never a newly generated debug key for each public update.

For iOS, install full Xcode (Command Line Tools alone is insufficient):

```sh
./scripts/build-mobile-ios.sh iphonesimulator
cd apps/mobile
flutter build ios --simulator --debug --no-codesign
```

A physical iPhone release for recipient re-signing can be built without a team:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
FLUTTER_BIN=/absolute/path/to/flutter/bin/flutter \
bash scripts/build-mobile-ios-device.sh
```

Run this from the repository root. The script stages iOS sources in a temporary
directory (avoiding cloud-folder Finder metadata), builds release/AOT, and writes
`artifacts/mobile-2.3.2/Sanser-iOS-2.3.2-device-unsigned.ipa`, a SHA-256 checksum and
a validation report. It rejects Simulator binaries and personal provisioning
profiles. No team, certificate or Apple password is needed to produce this file.
The IPA **must be re-signed by its recipient** before installing on a real iPhone.
It is not a TestFlight/App Store submission or a universal tap-to-install IPA.

See [iPhone installation and distribution](../../docs/ios-distribution.md) for
AltStore/SideStore, free-account limits, Xcode deployment and the official
TestFlight/App Store route. No personal team is committed.

The `Mobile Flutter` GitHub workflow checks Dart, builds Android, runs independent
SNR1 crypto fixtures, builds an unsigned iOS Simulator app and separately builds
a release iPhone IPA for recipient re-signing. Simulator artifacts are not
installable iPhone IPA files.

## Native checks

```sh
cargo test -p sanser-mobile-p2p
cargo clippy -p sanser-mobile-p2p --all-targets -- -D warnings
cmake -S native/mobile -B /tmp/sanser-mobile-test -DSANSER_MOBILE_TESTS=ON
cmake --build /tmp/sanser-mobile-test --parallel
ctest --test-dir /tmp/sanser-mobile-test --output-on-failure
```

This verifies the real desktop/mobile handshake over UDP loopback, fragmented
video callbacks, keyboard delivery, native view-only enforcement and teardown.
Rust tests negotiate against the desktop connectivity checker, forward real UDP
in both directions on the same reserved external port, and cancel an in-flight
check. PCM tests cover preroll, stereo sample conversion, underrun fading,
overflow and reset. These local tests do not establish WAN reliability or
confirm speaker/headphone audio quality on a real phone.

Cryptography uses Android JCA for existing SNV2 HMAC/AES and Google Tink 1.18.0
for SNR1 XChaCha20-Poly1305. iOS uses CommonCrypto and the Clibsodium binary package
from swift-sodium 0.9.1. Keys are session-bound; replay state changes only after
successful authentication. No cryptographic algorithm is implemented in Dart.

Dependencies retain their upstream licenses: Flutter/Dart (BSD), Opus (BSD),
OkHttp and Tink (Apache-2.0), libsodium (ISC), and Flutter secure storage (BSD).

## Optional iPhone native network diagnostic

Developer builds with `SANSER_NETWORK_DIAGNOSTICS` (or Debug) accept the explicit
launch argument `--sanser-probe-direct`. This runs the production native gather
boundary with a fresh temporary attempt, closes it, and writes
`Documents/sanser-direct-probe.json` inside the app sandbox. It records only
interface/candidate counts, STUN success and bounded native error text. It does
not contain endpoint addresses, account credentials or session keys. The mode
is excluded from an ordinary Release build.

Host candidate IDs include the OS interface index. iOS may expose the same IPv4
address on multiple cellular/virtual interfaces; these must remain distinct IDs
before endpoint deduplication, otherwise the candidate set rejects a collision
and aborts gathering even though the phone has working UDP connectivity.

## Media load and latency

Mobile requests a 6 Mbps receive ceiling by default (Settings offers 2/4/6/8/12
Mbps for the next session). Updated SNV2 hosts honor the authenticated `SVB1`
keepalive, warm-start at at most 4 Mbps and retain the requested FPS. Legacy
hosts ignore the optional extension and require a host update to enforce it.
The nominated Direct RTT seeds the mobile delay baseline when available.

A 250 ms same-transport control probe reports measured RTT and its minimum
baseline in optional authenticated `SVL1` feedback. Sustained excess RTT lowers
the encoder budget even without packet loss, with a cooldown and recovery hold;
healthy delivery slowly probes upward within the receiver ceiling. The wire
pacer uses that live budget and a two-datagram burst allowance. This controls
application load; it does not guarantee a fixed RTT on a congested ISP/Wi-Fi.

`sanser-rate-control-test` simulates a 200 ms receive delay without dropping UDP
packets, checks authenticated rate negotiation/backoff, measures actual video
send spacing after adaptation and verifies reliable input still arrives.

iPhone decodes H.264 on a dedicated VideoToolbox worker and coalesces decoded
images into one presentation slot. Busy UI work no longer drops compressed
reference frames. Android drains older decoded output before rendering the
newest image and continues polling while decode output is pending. Both keep
compressed recovery bounded. Run `bash scripts/test-mobile-video-ios.sh` on macOS
to exercise the production decoder with a real H.264 reference chain and a
deliberately blocked UI run loop.

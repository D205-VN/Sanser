# Sanser Mobile — Flutter 2.3.1 preview

Flutter/Dart provides the Android and iOS interface. A native C++ adapter reuses
Sanser's authenticated SNV2 transport, input delivery, frame reassembly and Opus
decoder. Compressed video and PCM never pass through Dart method channels.

Implemented in source:

- HTTPS account login, refresh tokens in Keychain/Android encrypted storage,
  stable installation/device identity and computer discovery.
- Request remote control or view-only access to a Windows/macOS host; approve or
  decline group invitations. View-only input is disabled by the native Peer.
- Native authenticated WSS relay, with the same session-bound SNR1 AEAD envelope
  as desktop; H.264 native rendering and 48 kHz system-audio playback.
- Absolute touch/drag, HID buttons and an ASCII keyboard; disconnect/release input
  on exit/background. Session state changes only after the native engine reports
  its first frame; RTT comes from actual authenticated probes.

Initial preview limits:

- **WSS Relay only.** Direct/ICE and UDP Relay are not implemented for mobile yet.
  No 5–15 ms latency promise is made for this transport.
- **Group voice and microphone capture are not enabled on mobile.** System audio
  is separate and supported in source. The UI hides unavailable voice actions.
- No mobile host, gamepad, full IME/Unicode text injection or background control.
- Requires the Sanser **2.3.1 server** to register this client version; desktop
  Windows/macOS hosts must expose the bidirectional SNV2 engine.
- Builds/unit tests do not replace real Android/iPhone-to-host testing.

## Build

Pinned Flutter: **3.47.7**, revision
`abaf9c523780a608bd46686fd5e53740a07077f8`. Android: JDK 17, SDK 36, NDK
28.2.13676358, CMake 3.22.1. Android 8+ / iOS 15+.

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

A physical iPhone build uses `./scripts/build-mobile-ios.sh iphoneos` and an Apple
signing team configured locally. No personal team is committed. Without a paid
Apple Developer membership, TestFlight/App Store distribution is unavailable.

The `Mobile Flutter` GitHub workflow checks Dart, builds Android, runs independent
SNR1 crypto fixtures and builds an unsigned iOS Simulator app. Simulator artifacts
are not installable iPhone IPA files.

## Native checks

```sh
cmake -S native/mobile -B /tmp/sanser-mobile-test -DSANSER_MOBILE_TESTS=ON
cmake --build /tmp/sanser-mobile-test --parallel
ctest --test-dir /tmp/sanser-mobile-test --output-on-failure
```

This verifies the real desktop/mobile handshake over UDP loopback, fragmented
video callbacks, keyboard delivery, native view-only enforcement and teardown.

Cryptography uses Android JCA for existing SNV2 HMAC/AES and Google Tink 1.18.0
for SNR1 XChaCha20-Poly1305. iOS uses CommonCrypto and the Clibsodium binary package
from swift-sodium 0.9.1. Keys are session-bound; replay state changes only after
successful authentication. No cryptographic algorithm is implemented in Dart.

Dependencies retain their upstream licenses: Flutter/Dart (BSD), Opus (BSD),
OkHttp and Tink (Apache-2.0), libsodium (ISC), and Flutter secure storage (BSD).

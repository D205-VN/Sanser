# Local Voice Lab SFU validation

This is a **functional integration test**, not an acoustic-quality or release
certificate. It runs only when explicitly invoked. No microphone, speaker,
Sanser installation, personal account, cloud SFU or production database is used.

Pinned tools used for the local run:

- LiveKit server `v1.13.11`, built from its official source release.
- Go `1.27.3`, official Darwin ARM64 toolchain.
- LiveKit server SDK Go `v2.19.0`; `peers/go.mod` and `go.sum` pin dependencies.
- PostgreSQL `17.11`, extracted from
  `@embedded-postgres/darwin-arm64@17.11.0-beta.19` (verified package integrity).
- FFmpeg with `libopus` creates a 440 Hz synthetic tone; it records nothing.

Keep toolchains/caches in a local temporary directory if Documents is managed by
iCloud: a dataless executable can appear to exist but fail or execute as an empty
shell file. Download/build tools separately; the runner does not install them.

Build the peer driver from this directory:

```sh
cd apps/voice-lab/testing/peers
GOTOOLCHAIN=local go build -mod=readonly -trimpath -o /private/tmp/sanser-peer-test .
```

Run from the repository root, supplying your explicit tool paths:

```sh
python3 apps/voice-lab/testing/run-sfu-local.py \
  --postgres-bin /absolute/local/postgres/bin \
  --livekit-server /absolute/local/livekit-server \
  --peer-driver /private/tmp/sanser-peer-test \
  --target-dir /private/tmp/sanser-voice-cargo \
  --output artifacts/voice-lab-sfu-local \
  --require-production-revocation
```

Use `--sample-seconds 120` (or 10–600) for a longer sample. The tone file and
test deadlines scale with duration. Each remote Opus track must keep receiving
packets (no gap over three seconds after startup); the report includes maximum
userspace RTP receipt gaps. Heartbeats and the production closure worker run
throughout, rather than postponing live authorization checks until the end.
There is no mic/speaker playback or acoustic pass criterion in this fixture.
Run the short trial too: the original 60-second grant expires during a long
sample, so long-sample denial alone cannot prove original-token revocation.

Rust dependencies must already be fetched: the runner uses `--locked --offline`.
The output directory must be writable; ports `55439`, `17880`, `17881`, gateway `17883` and UDP
`17882` must be unused. The runner initializes a fresh cluster under
`/private/tmp`, binds services to loopback and cleans up only its own processes
and temporary directory even if the test fails. Synthetic join tokens are passed
through a private file inside a mode-0700 directory, never command-line args or
logs. Test keys/passwords are fixed synthetic fixtures, unrelated to accounts.

The checks are:

1. Run the database API suite, including closure retry, restart, lease recovery,
   concurrent-worker claims, group deletion and bounded closed-room retention
   that preserves pending/live/leased work.
2. Create a synthetic account/group through Sanser's real API handlers; admit one
   host and two accepted viewers with scoped credentials.
3. Connect three SDK peers. Each publishes Opus microphone-source packets and
   receives two remote Opus tracks for at least ten seconds. Counts demonstrate
   transport activity, **not** decoding/playout quality, loss rate or latency.
4. Stop sharing through the API and require all three SFU peers to disconnect.
5. Obtain an actual SFU refresh token through a signaling-only probe before the
   media trial, then attempt both cached and refreshed-token rejoin through the
   gateway after closure. Both must fail; the strict flag makes replay fail the
   runner even if other functional checks pass.

Current result: the private-SFU/gateway topology rejects both token types and
closes all three peers. The earlier raw-SFU replay failure is retained in the
phase-5b3 evidence: exposing its HTTP/WS port publicly would still bypass this
boundary. `gatewayRevocationVerified: true` certifies the exercised local
scenario, not a deployed production network. `hardwareAudioVerified` stays false.

Database tests also exercise immediate rejection when offline/logged out/stopped,
reconciliation after logout, and a new login that does not revive the old token.
Mic/speaker quality, Windows and Flutter runtime, output switching, echo and long
sessions still need physical tests. Desktop handoff/consent/UI and native sound
publication gates have separate fake-device and UDP tests.

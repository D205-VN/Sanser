# Local network simulation and log stress test

The user has no Windows computer available. These results measure the **new SNV2
transport on a single Mac**, with UDP impairment injected by a loopback proxy.
They are not a Windows remote-control test, a physical LAN/WAN test, or measurements
of the existing Mac-client/Windows-host legacy path. WSS relay is not benchmarked.

## Setup

- Synthetic encoded-video payload: 20 Mb/s, 60 frames/s, 1920×1080 metadata.
  Payloads are bytes, not actual H.264 frames; capture, encoding, decoding and display
  are excluded. This benchmark cannot measure screen-to-screen latency.
- Mouse: 120 events/s. Reliable keyboard: 20 events/s. Neither is injected into the OS.
- Three runs per scenario, eight seconds of traffic per run, plus startup and two
  seconds for pending traffic to drain. Fixed random seeds are recorded in raw data;
  OS scheduling still changes packet interleaving between runs.
- Delay and jitter apply independently in each direction; bandwidth is full duplex.
  The proxy drops packets when its modeled serialization queue exceeds 90 ms.
- Raw observations and test-machine OS: [native-network-local.json](benchmarks/native-network-local.json).

| Simulated scenario | Network model | Keyboard arrival p95, one way | Complete video frames delivered/s |
| --- | --- | --- | --- |
| LAN | 1 ± 0.25 ms; 100 Mb/s; no random loss | 1.41–1.43 ms | 59.99 |
| WAN | 25 ± 8 ms; 40 Mb/s; 0.5% random loss | 32.29–32.57 ms | 11.62–18.62 |
| Congested WAN | 60 ± 20 ms; 15 Mb/s; 2% random loss | 77.95–79.57 ms | 0–0.25 |

The figures above are ranges across the three runs. P95 means 95% of observed
keyboard events arrived within that time. All 1,440 reliable keyboard events were
received in order across the nine runs. Mouse position updates are intentionally
allowed to drop when stale. Transport processes stayed alive, which **does not**
mean video performance passed: the WAN cases clearly failed to maintain the
requested frame rate. Zero-valued frame latency fields in runs with no received
frames mean there were no samples, not zero latency.

## Findings and fixes

1. Mouse packets could arrive out of order and overwrite a newer pointer position.
   In the initial single-run WAN scenarios, 101 and 373 stale positions were applied.
   The receiver now uses a monotonically increasing mouse sequence within a peer
   generation. After the fix, all nine runs applied zero stale mouse positions.
   Key/button delivery retains its separate reliable ordering.
2. The shell piped new engines' stderr but previously read it only after process
   exit. A chatty encoder could therefore fill the OS pipe and stall media work.
   Stderr is now drained by a background reader from startup. The error tail is
   bounded to 8 KiB per process. Optional debug files are capped at 2 MiB per engine
   launch; files are truncated when a new session starts, and disk logging remains
   off unless `SANSER_SIDECAR_DEBUG_LOG=true`. Stdout is discarded by the launcher.
   The last 2,048 error characters remain available in the UI after exit.
3. Rust stress tests generated 4 MiB of stderr, verified the child completed without
   waiting for a pipe read at exit, retained its final error and enforced the file
   cap. These tests demonstrate output handling, not Windows encoder performance.
4. The new video path is still fragile under loss. A frame spans many UDP packets;
   incomplete frames are dropped, subsequent dependent frames await a keyframe,
   and keyframe requests are throttled. The 20 Mb/s synthetic payload also exceeds
   the congested scenario's 15 Mb/s capacity before protocol overhead. This path
   needs bounded video loss recovery and congestion-driven bitrate adaptation
   before it can be described as smooth across networks. Those changes are not
   implemented by this test/fix.

Validation after the fixes: 21 desktop Rust tests, desktop clippy, and five native
CTest suites passed, including encrypted loopback and a separate synthetic
VideoToolbox codec round trip. The codec test is separate from the network timing
results above. Real Windows capture/input, GPU load, long sessions, changing Wi-Fi,
NAT traversal and remote relay performance remain unverified.

## Reproduce

```sh
cmake -S native/desktop -B /tmp/sanser-desktop-native-build -DBUILD_TESTING=ON
cmake --build /tmp/sanser-desktop-native-build --parallel
python3 scripts/benchmark-native-network.py \
  /tmp/sanser-desktop-native-build/sanser-desktop-transport-bench \
  --seconds 8 --repeats 3 --output /tmp/sanser-network.json
node scripts/run-cargo.mjs test -p sanser-desktop --lib
ctest --test-dir /tmp/sanser-desktop-native-build --output-on-failure
```

The benchmark exit code checks peer survival, reliable input and monotonic mouse
positions; it does not certify video quality. Inspect the delivered frame counts.

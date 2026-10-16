# Compare the working 2.2.1 Mac → Windows baseline

The reported observation is 10–15 ms Wire RTT with 2.2.1 versus about 199 ms
with a newer build, both labelled Direct. Preserve the working installation and
its settings until a controlled comparison identifies the difference.

## Source audit

Comparing tags `v2.2.1` and `v2.2.7`, **25 implementation files are identical**
across these paths (version headers and test files excluded):

- `native/client-macos/src`: legacy Mac receive, decode, render, input and RTT.
- `native/host-windows/src`: legacy Windows capture, encode, pacing and input.
- `native/common`: shared latency, timing, adaptation and waiting policies.
- `native/protocol`: shared framing, authentication and STUN implementation.

The server still selects `legacy` for ordinary Mac-client/Windows-host sessions
in `apps/server/src/routes/sessions.rs::native_wire_protocol`. Group sharing uses
SNV2 and is a different pipeline. The new SNV2 Windows viewer and Mac host fixes
must not be rolled back on the assumption that they replaced this legacy path.

Source parity does **not** mean the entire programs or their measured performance
are identical. Shared backend/linking code, libraries, build environment, UI,
process lifecycle, settings and the actual nominated route must also be checked.
Copying the same legacy source into another "2.2.1 mode" would change no algorithm
and would provide no demonstrated latency fix.

## Saved measurements inspected on the Mac

The following numeric results come from automatic reports already on the Mac,
not a new two-computer benchmark. The installed application was 2.2.1 when
inspected. Its installation and preferences were left unchanged.

| Report | Role | Mode | Requested settings | Retained live Wire RTT | Probe median |
| --- | --- | --- | --- | --- | --- |
| 2.2.1, Oct 15 20:28 +07 | Client | Direct, Ultra | 60 FPS, 25 Mbps | 16 samples: P50 12.9 ms, P95 31.8 ms | 13.15 ms |
| 2.2.1, Oct 15 20:35 +07 | Client | Direct, Ultra | 60 FPS, 25 Mbps | 11 samples: P50 14.8 ms, P95 200.9 ms | 12.84 ms |
| 2.2.4, Oct 15 18:27 +07 | Host | Direct, Ultra | 60 FPS, 25 Mbps | No live measurements | 198.91 ms |

These reports retain at most 120 samples shared across metric sources. The
percentiles describe **only retained Wire samples**, not the complete session.
In particular, the second report contains occasional high RTT even in 2.2.1.
The host-role 2.2.4 probe is not a matched Mac-client live RTT comparison.
No saved 2.2.7 live client report was available in the inspected report directory;
the user's current 199 ms observation remains to be captured and compared.

Existing reports do not persist the selected candidate endpoints, protocol or
peer engine version. "Direct" alone cannot distinguish physical LAN, public
STUN/mapped UDP or a VPN/overlay path. A 199 ms probe before media is a useful
lead, but does not by itself prove NAT, ISP, OS scheduling or a particular bug.

## Reproduce the read-only comparison

From the repository root on the Mac:

```sh
node scripts/compare-latency-baseline.mjs \
  --baseline v2.2.1 --current v2.2.7 \
  --reports "$HOME/Library/Logs/com.sanser.desktop/connections" \
  --output /tmp/sanser-latency-baseline.json
```

`--reports` also accepts an automatic report or a Diagnostics export JSON from
either computer. The output strips machine names, account details, session IDs,
endpoints, arbitrary log fields and credentials. It reports missing metrics as
`null`; it never substitutes nomination RTT or a whole-session maximum for live
Wire RTT. It does not launch anything or change routes, settings or installations.

An initial sanitized snapshot of the inspected reports was saved locally at
`/private/tmp/sanser-latency-baseline.json` to preserve the comparison before
report retention rotates old measurements. This local artifact is not committed.

## Controlled two-computer test

1. Keep the **same Windows host, network/VPN state, physical adapters and route
   endpoints**, 60 FPS, 25 Mbps and Ultra profile as the working 2.2.1 session.
   Test ordinary Connect, with group sharing and reverse-direction streaming
   stopped. Record both peer versions, codec and requested resolution.
2. For 2.2.1, collect at least 60 seconds of idle and active desktop measurements
   and export Diagnostics while connected. Capture Connection details as well:
   selected candidate types/endpoints, advertised local interface and probe RTT.
   Those endpoints are necessary because older automatic reports omit them.
3. Quit both apps completely between runs. Repeat on the newer version with
   matching settings; save its export while connected. A mixed-version run is
   not the same A/B experiment. Do not erase permissions or preferences to test.
4. Repeat 2.2.1 once more (A–B–A) to check whether route/network conditions changed
   over time. Direct on both runs is insufficient evidence of a matching path.
5. Compare **probe / raw UDP echo / live Wire / application queues / input ACK**
   separately. If the newer probe is already 199 ms, investigate path selection
   and receive scheduling before video tuning. If probes match but only live
   RTT increases, compare echo, socket queues and media-load conditions.

Retain the actual samples/counts and route metadata. No successful reduction
from 199 ms to 10–15 ms is claimed by this source audit or its unit tests.

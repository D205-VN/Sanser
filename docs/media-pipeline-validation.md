# Part 5: video recovery and comparable desktop measurements

Included in 2.2.13 with parts 1–4. OS capture, encoding, decoding, presentation and
input remain native; this does not force ordinary Mac→Windows onto SNV2 or remove
its Legacy fallback. Android/iOS, group audio, seamless route migration and a
complete transport rewrite are not part of this release.

## Bounded SNV2 decode recovery

The shared decoder on both Windows and macOS now checks **local queue age** as
well as queue length/bytes. A waiting reference chain expires after 50 ms in
Ultra or 100 ms in standard mode. One-time decoder initialization retains its
bounded 30-picture chain for at most 500 ms; steady-state capacity remains three
pictures, and memory remains bounded to 8 MiB. These are maximum recovery ages,
not a fixed playout delay. Complete usable frames are submitted immediately.

An incoming fresh IDR replaces the older queued chain even when that chain is
below capacity. Expired or overflowed P-pictures are discarded as a chain; the
worker resets its codec and waits for a valid fresh IDR. An arbitrary newest
P-picture is never treated as an independent image. Keyframe requests are limited
to once per 100 ms in Ultra or 300 ms in standard mode. An unchanged idle desktop
does not expire merely because no new capture arrives: only waiting decode work
has an age budget.

Codec reset/cleanup remains on its owning decode thread. Exceptions in decoder
reset or the recovery callback now become terminal errors for the viewer instead
of escaping a worker and terminating the process. The shell reports the failure;
part 3 does not automatically reopen an authentication/decoder failure.
An in-flight OS/GPU decoder call cannot be cancelled, so this is not a guarantee
of a sub-budget decode call or end-to-end latency.

`decodeQueueExpired` counts discarded waiting pictures. `decodeQueueMaxMs` is the
largest measured enqueue-to-decode-start duration in that reporting interval,
not the age of expired pictures or a kernel/network timestamp.

## Four-direction measurement matrix

`scripts/performance-matrix.mjs` reads **explicitly supplied numeric reports**
from Diagnostics exports. It handles both Legacy and SNV2 and creates 32 coverage
cells: four viewer/host pairs × Direct LAN, Direct WAN, UDP relay, WSS relay ×
standard/Ultra. Standard means the native non-Ultra setting, not a claim that all
quality presets or network Auto/Direct modes are identical.

Example manifest saved next to exports from the two computers:

```json
{
  "cases": [
    { "viewer": "macos", "host": "windows", "network": "wan", "file": "mac-viewer.json" },
    { "viewer": "windows", "host": "macos", "network": "wan", "file": "win-viewer.json" },
    { "viewer": "windows", "host": "macos", "network": "wan", "role": "host", "file": "mac-host.json" }
  ]
}
```

```sh
node scripts/performance-matrix.mjs --manifest /tmp/cases.json --output /tmp/matrix-new.json
node --test scripts/performance-matrix.test.mjs
```

The output must be a new file; input exports, the manifest and an existing output
are never overwritten. Each case defaults to client-role reports. Optional
`sessionId` selects a specific session internally without exporting its ID.
The recorded local OS is checked against the annotation. Peer OS, network scope,
peer version and route endpoints are not independently verified. Direct with an
unknown LAN/WAN annotation remains unclassified; missing relay transport is not
silently called UDP.

Reports retain their own protocol, settings, role, version and stage P50/P95/max.
Schema-2 rolling summaries are preferred; older reports use retained numeric
observations. No percentiles from different reports are pooled. Missing stages
stay null and untested cases stay `not-observed`. A populated case is explicitly
`observations-only`, never an automatic passing performance certification.
No machine names, account details, session IDs, IP addresses, packet contents,
keystrokes or credentials are copied into this artifact. It does not read token
stores, launch engines or change network settings.

For physical testing keep FPS, bitrate, codec, resolution, adapters and route
fixed; collect both idle and active desktop measurements for at least one minute.
Compare control RTT, wire/echo where available, receipt ACK, host input queue/work,
capture/encode and decode/presentation separately. Test all four directions and
group viewers, focus loss, held keys at Disconnect and route loss/reconnect.
Synthetic codec/loopback tests are separate from these real-device tests.

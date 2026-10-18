# Part 3: bounded adaptive video FEC

Development on `Version-2.0.0` after continuous Direct standby measurement.
Included in 2.2.16. No new server, port or external runtime dependency is required.

## Scope and placement

`sanser-p2p::fec` is a shared Rust protection engine used by the existing Direct
UDP router and UDP Relay bridge. It recognizes original SNV2 Video/stream 1 and
authenticated encrypted Legacy SNU2 video datagrams. Plain SNU1, manual native
launches without the Direct router, WSS/TCP and non-video traffic are unchanged.
Both ordinary control directions and independent group viewers can use it when
both local bridges support the capability. Group audio uses its own unprotected-by-FEC priority lane.
This is video packet recovery, not Opus in-band FEC or RTP audio Reed–Solomon.

Original native datagrams are sent/forwarded immediately and unchanged. A small
FEC layer protects **opaque native ciphertext**, including its original header,
authentication tag and sequence. Recovered datagrams enter the same native
UDP path, which still verifies its own authentication/replay/generation rules
before decrypting or decoding. No screenshot, video decode/re-encode, input
injection or webview media callback is added.

## Code and deadlines

The first implementation is systematic **XOR**, one parity datagram for a block
of **8–16 original video datagrams**. It reconstructs one missing datagram per
block, including variable lengths. Two missing datagrams cannot be fabricated:
keep the block briefly for reordered data, then leave residual loss to existing
native NACK/retransmission/keyframe handling. There is no Reed–Solomon/multiple
parity claim in this version.

Parity is generated when a block fills. A timer may flush an eligible partial
block after 8 ms; fewer than eight datagrams are discarded from FEC grouping,
not held waiting for the next frame. Native video has already been sent. Stop
partial grouping on generation/sequence discontinuity. Groups older than 20 ms
cannot generate parity. No original frame/repair/render deadline is extended.

Receiver caches at most **256 datagrams**, expires cached data and pending
parity after **60 ms**, and keeps at most **32 pending blocks**. At high rates,
the count limit can expire data earlier. Recover at most four datagrams per
receive call. Input/audio/control bypass cache pruning and XOR work entirely.
No parity write blocks or creates a new retry queue. A local send/drop signal
immediately pauses protection. Each group viewer has its own keys, budget,
cache, feedback and counters; FEC does not request a shared encoder keyframe.

## Negotiation and authentication

`SFV1` optional records are consumed by the bridge and never delivered to a
supporting native engine. Originals do not acquire a mandatory FEC header.
Authenticated capability messages exchange random per-bridge epochs and echo
the peer epoch. Both directions must confirm before parity can be sent. Old
peers never confirm and continue with the original native media path.

The record has a 48-byte version/kind/family/count/epoch/sequence/generation/base
header, followed by bounded data and a **16-byte HMAC-SHA256 tag**. Parity entries
carry a u16 sequence delta and u16 original length, then padded XOR bytes.
Separate directional MAC keys bind the signaling session and expected device;
Direct derives from its authenticated probe secret, while UDP Relay derives
under a distinct domain from its existing end-to-end frame secret. No key,
challenge or media content is added to diagnostics. Pin the peer bridge epoch,
reject retired native generations and enforce a 512-record replay window.
Restored data must match the promised native family, generation, sequence and
length; native cryptographic verification remains the final check.

UDP Relay puts every FEC record **inside** existing SNR1 end-to-end encryption
and authenticated UDP relay allocation framing. The server forwards ciphertext;
it does not need native media keys or an FEC-aware deployment. Direct does not
introduce a new plaintext media channel: parity is computed on ciphertext.

## Adaptation and congestion guard

Initially **off**. A receiver reports numeric window counters once a second;
missing native video sequences get an 80 ms reorder grace before loss accounting.
Reconstruction is reported separately and contributes to protection demand.
An early reconstruction during reordering is useful work, not proof that the
original datagram would never have arrived. These counters are not an ICMP
measurement or an exact attribution of loss to the ISP.

For at least 64 observed datagrams per valid window and two consecutive demand
windows, target parity budgets are:

| Loss/reconstruction demand | Maximum parity wire budget |
| --- | --- |
| <0.5% | Off after three healthy windows |
| 0.5–<1% | 8% |
| 1–<3% | 12% |
| 3–<12% | 15% |
| >=12%, >=3% late, or reported local drops | Pause protection |

A byte-credit limiter charges **complete parity headers and transport overhead**,
not only XOR payload. Credit is earned only from originals actually sent, is
capped to two datagrams and cleared on pause/change. A failed write may consume
credit conservatively. Tiny/partial blocks, insufficient credit or MTU overflow
skip parity. It cannot spend more than 15% of earned video wire bytes; capability
and numeric feedback add a small, separately bounded control cost.

Direct also pauses FEC if live same-socket probe median rises over the running
minimum by max(20 ms, 50%). Any local overload or severe feedback pauses for five
seconds. A path switch clears stale feedback/credit and waits for new evidence.
Feedback older than three seconds disables parity. This is a conservative
congestion guard, **not a bandwidth estimator or a replacement for bitrate
control**. FEC cannot fix capture stalls, a slow TCP relay or a 200 ms route.

FEC records are capped at 1400 bytes on Direct and 1348 before UDP Relay's
124-byte envelope, so relay UDP payload stays <=1472 for a 1500-byte IPv4 MTU.
Block size adapts to available record space. Smaller VPN/Internet path MTUs
still require real validation/PMTU work; this is not universal fragmentation
avoidance or proof of Internet UDP ingress on Render.

## Diagnostics and validation

`fec-health` shows per-engine/session negotiation, local sender budget, parity,
reconstruction, expiry, rejection, budget skips and congestion pauses. Keep eight
latest records; reject older engine-generation updates and mark observations
historical after five seconds. These are counts/percentages, not latency stages.
Numeric-only `FEC_HEALTH` is also included in bounded connection reports;
additional group peer UI snapshots remain available in Export diagnostics.

Tests cover both native wire families, every single erasure position, variable
sizes, parity/data reordering, unrepairable two-loss blocks, generation/epoch
reset, tampering, feedback replay, malformed/oversized data, bounded caches,
every budget tier, old peers, stale feedback, local overload and route changes.
End-to-end loopback tests intentionally discard video in both Direct and
end-to-end encrypted UDP Relay, verify that discarded datagrams reappear byte
for byte, and send unchanged input/audio alongside video. The fixture uses
synthetic native-shaped data, not real capture or injected OS input. Windows CI
runs both core FEC and live UDP fixtures, besides normal native/frontend checks.

Remaining acceptance: two physical machines in both directions, A/B FEC on/off
under controlled random/burst loss and changing load; P50/P95 display/input
latency, bitrate, CPU, capture/encode work and total wire overhead. Test group
viewers independently, relay availability, VPN MTUs, rekey, UAC/pause, sleep and
reconnect. Do not claim that synthetic recovery proves every user's freezes,
crackle or network latency is fixed.

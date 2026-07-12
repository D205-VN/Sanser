# SNV2 protocol

SNV2 is Sanser protocol version 2. All integers use network byte order (big-endian). A receiver validates the complete fixed header before allocating or authenticating payload memory.

## Fixed header (80 bytes)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | ASCII magic `SNV2` |
| 4 | 1 | protocol version `2` |
| 5 | 1 | header length `80` |
| 6 | 1 | packet type |
| 7 | 1 | canonical priority |
| 8 | 2 | flags |
| 10 | 2 | reserved, must be zero |
| 12 | 16 | non-zero session ID |
| 28 | 4 | stream ID |
| 32 | 8 | sequence number |
| 40 | 8 | frame ID |
| 48 | 8 | monotonic timestamp in microseconds |
| 56 | 4 | payload length |
| 60 | 4 | key ID |
| 64 | 16 | authentication tag |

The authentication tag is the first 16 bytes of HMAC-SHA256 over bytes `0..64` followed by the payload. Session direction keys are 32 bytes and transmit/receive keys must differ. Implementations use platform or audited cryptographic libraries and constant-time tag comparison.

Defined flag bits are:

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `KEY_FRAME` | The video frame can be decoded without an earlier frame. |
| 1 | `END_OF_FRAME` | This is the last fragment of a video frame. |
| 2 | `RETRANSMITTED` | This packet is a retransmission. |
| 3 | `ACK_REQUIRED` | The sender requests an acknowledgement. |
| 4 | `DISCONTINUITY` | The stream has a discontinuity before this packet. |
| 5 | `START_OF_FRAME` | This is the first fragment of a video frame. |

Every video frame has exactly one `START_OF_FRAME` fragment and one
`END_OF_FRAME` fragment; a one-packet frame sets both. Reassembly completes
only when every sequence in the inclusive, explicitly marked range is present.
It never infers the start from the lowest fragment received, because that could
turn loss of the real first fragment into an apparently complete corrupt frame.
Until both boundaries are known the receiver cannot safely generate a complete
NACK range, and the bounded expiry policy drops the incomplete frame.

`START_OF_FRAME` uses a previously undefined flag bit and does not change the
fixed header layout or protocol version. Senders and strict flag-validating
receivers must nevertheless be upgraded together: a legacy sender that omits
the start marker cannot complete a frame in the corrected assembler, while a
legacy strict receiver may reject bit 5 as unknown.

## Packet classes and lanes

SNV2 defines handshake, authentication, video, audio, mouse move/button,
keyboard, gamepad, clipboard, network/encoder/decoder feedback, NACK, keyframe
request, keepalive, disconnect, error, and acknowledgement packets. Packet type
`18` is `Acknowledgement`.

Canonical lanes are:

1. Reliable input: keys, mouse buttons, disconnect.
2. Realtime input: mouse movement and gamepad; latest state wins.
3. Audio.
4. Video control: handshake/auth/feedback/NACK/keyframe/keepalive/acknowledgement.
5. Video payload.
6. Diagnostics and clipboard metadata.

The global payload maximum is 1 MiB with smaller per-type limits. Packet length
must equal `80 + payload_length`. Unknown version/type/flag bits, wrong priority,
zero session, reserved bits, oversize, endpoint mismatch, replayed sequence, and
invalid tag are rejected.

## ACK/SACK payload (16 bytes)

`Acknowledgement` is a fixed-width, bounded wire primitive. The packet header's
`stream_id` identifies the stream being acknowledged. The acknowledgement
packet's own header sequence remains independently authenticated and replay
checked.

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 8 | cumulative sequence |
| 8 | 8 | selective mask |

Both fields are unsigned 64-bit big-endian integers. The cumulative sequence
acknowledges itself and every earlier sequence in the same session, stream,
direction and key generation. Selective-mask bit `i`, with bit `0` as the
least-significant bit, acknowledges `cumulative_sequence + 1 + i`; therefore
bit `0` represents the immediately following sequence and bit `63` represents
`cumulative_sequence + 64`. Sequence numbers never wrap within a key
generation. Senders rekey or close before exhaustion.

A receiver emits no acknowledgement until it has a cumulative base sequence;
there is no sentinel value for “nothing cumulative yet”. An acknowledgement
packet never sets `ACK_REQUIRED`, preventing acknowledgement loops.

An acknowledgement payload must contain exactly 16 bytes and uses the
Video-control lane. This gives ACK traffic precedence over video payload and
diagnostics without allowing it to jump ahead of realtime input or audio.
ACK/SACK generation, retry timers and ordered delivery are intentionally not
wired into the native sidecars yet; this section defines only their shared wire
contract.

## Realtime rules

- Reassembly and retransmission windows are bounded by bytes, frames, and age.
- An incomplete expired video frame is dropped; it is not allowed to grow latency.
- NACK never retransmits media from an older key generation.
- Video payload does not share a blocking queue with input or audio.
- Session teardown releases all pressed input state.

The portable C++ codec and tests live in `native/protocol`; the canonical Rust codec lives in `crates/sanser-protocol`.

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

## Packet classes and lanes

SNV2 defines handshake, authentication, video, audio, mouse move/button, keyboard, gamepad, clipboard, network/encoder/decoder feedback, NACK, keyframe request, keepalive, disconnect, and error packets.

Canonical lanes are:

1. Reliable input: keys, mouse buttons, disconnect.
2. Realtime input: mouse movement and gamepad; latest state wins.
3. Audio.
4. Video control: handshake/auth/feedback/NACK/keyframe/keepalive.
5. Video payload.
6. Diagnostics and clipboard metadata.

The global payload maximum is 1 MiB with smaller per-type limits. Packet length must equal `80 + payload_length`. Unknown version/type, wrong priority, zero session, reserved bits, oversize, endpoint mismatch, replayed sequence, and invalid tag are rejected.

## Realtime rules

- Reassembly and retransmission windows are bounded by bytes, frames, and age.
- An incomplete expired video frame is dropped; it is not allowed to grow latency.
- NACK never retransmits media from an older key generation.
- Video payload does not share a blocking queue with input or audio.
- Session teardown releases all pressed input state.

The portable C++ codec and tests live in `native/protocol`; the canonical Rust codec lives in `crates/sanser-protocol`.

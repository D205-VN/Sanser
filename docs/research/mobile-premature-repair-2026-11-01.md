# Mobile premature repair investigation

After 2.3.2, local Mac host reports showed Direct sessions with ~10–12 ms
encode/publish work, but hundreds/thousands of queued repairs and repeated video
queue drops. These counters alone do not establish packet loss or user-visible
end-to-end latency.

A new real-UDP regression reproduces the mismatch without intentional packet
loss: the mobile receive path initialized reassembly at 12 Mbps while the host
actually paced a 128 KiB IDR at a negotiated 2 Mbps. The receive repair deadline
could expire while original fragments were still in the host's sender queue.
Before the fix the test queued 96 needless repair datagrams. Those repairs share
the paced video budget, and unique retransmissions also counted as lost fragments
in delivery feedback, which could lower bitrate and amplify the mismatch.

Changes:

- Track original fragment dispatch per frame without taking the capture/cache
  mutex from the sender. Ignore NACK indices whose original has not yet been
  attempted. Transient UDP send failures still remain eligible for real repair.
- An optional authenticated `SVR1` keepalive reports the current host video
  bitrate. Supporting mobile clients update only their serialization estimate;
  completed frames are never held. Stale rate messages cannot overwrite newer
  ones, and the generation fence resets the rate sequence.
- Before rate feedback arrives, the mobile reassembler starts at its bounded
  receive ceiling/warm-start rate. Desktop peers retain existing behavior.
- Diagnostics allowlist numeric bitrate and premature-repair counters; session
  keys, endpoint addresses and frame/input contents remain excluded.

`sanser-mobile-pacing-test` checks the lossless IDR path, continued P-pictures,
priority keys, then deliberately drops a real original datagram and verifies
repair resumes the reference chain. The original lossless reproduction queued
96 repairs; a post-fix run queued 0, with no video queue drops. A small tail race
from kernel/receiver scheduling is allowed by the regression; it must never
turn into a block of premature repairs. Real phone/WAN smoothness still needs
measurement; these results do not claim the user's reported lag is fully fixed.

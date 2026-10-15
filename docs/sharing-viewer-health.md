# Slow viewer handling (2.2.5)

Sharing still uses one encoder and at most three authenticated, view-only peers.
This policy isolates a viewer with sustained delivery problems; it is separate
from Direct/Relay route selection and does not claim to fix WAN RTT.

The native host evaluates the same counters exported in `SNSHARE_PEER` directly,
under each delivery's lock. Decisions do not wait for stdout parsing or the UI.
Monotonic half-second windows use counter deltas rather than lifetime ratios:

* At least ten submitted frames with replacements/reference drops at or above
  35%, or fresh send-queue drops, indicate local delivery pressure.
* Authenticated `SVF1` feedback also contributes sustained delivery deficit/loss.
* Six seconds of continuous pressure, or eight seconds of continuous 2-second
  keyframe backoff, pauses this viewer's fresh video.
* A host sampling gap over two seconds breaks evidence of sustained pressure.

While paused, the host discards that viewer's pending/latest video and queued
fresh video datagrams. It retains the authenticated connection, keepalives,
control, and the bounded repair queue/cache (150 ms). It ignores fresh keyframe
demands from the paused viewer and excludes it from the shared bitrate minimum.
If every viewer is paused, capture/encoding need not run until a probe is due.
No viewer is removed, and its account consent and session remain intact.

Recovery first waits five seconds, then allows a bounded three-second video
probe. A fresh IDR is required through the existing coalesced keyframe gate.
The probe cannot reduce healthy viewers' bitrate. Rejoining requires at least
three fresh clean feedback windows spanning 1.5 seconds, positive delivered
video bytes, and completion of a frame newer than the last frame submitted
before the probe. Zero counters, a live keepalive, and partial fragments alone
are not recovery evidence. Failed probes wait 10, 20, then at most 30 seconds.

An optional authenticated `SVP1` control feedback message carries the last video
frame reassembled and accepted by the client frame callback. It is sent with
the existing half-second feedback cadence, retains the `SVF1` format, and is
ignored by older hosts. It proves packet/frame progress, not display scan-out.
Both native apps should be updated for automatic rejoin; older clients without
this progress feedback remain paused after failed probes and can be removed and
invited again. This avoids declaring unverified recovery.

When no healthy viewer remains, a recovery probe may use its last recommended
bitrate; otherwise probing every weak link at the full source target could
prevent all viewers from ever recovering. With healthy viewers present, only
active viewers determine the shared encoder budget. After verified rejoin, the
viewer again participates in the common minimum and normal bitrate recovery.
There are no separate quality tiers or per-viewer encoders.

`SNSHARE_PEER` adds `videoState=active|paused|probing`, `pauses`, and `backoffMs`.
The desktop observes at most three UUID-keyed statuses, expires samples after
five seconds, and exposes only members of the current native sharing group.
The host sharing panel explains pause/probing, distinguishes a stopped
connection, and excludes paused/probing viewers from the watching count. Stop,
remove, and logout clear local status. No credentials or screen contents enter
the status API.

Validation covers virtual-time thresholds, silence/partial-fragment recovery
rejection, retry cooldown, control/repair lane preservation, parser bounds,
store lifecycle, and UI states. The authenticated UDP loopback test constrains
one viewer to 1 Mb/s, verifies pause and bitrate exclusion while another viewer
receives frames, then lowers offered data to demonstrate completion-verified
automatic rejoin. Real Windows/Mac capture and impaired Internet links still
require testing on physical devices.

# Viewer close and incoming access alerts

Packaged in 2.2.20. Update both installed desktops to receive these changes.

## Disconnect lifetime

Closing a native remote viewer ends only its session and sharing membership.
The desktop now retains `SNV2_VIEWER_CLOSED` / `SNU1_VIEWER_CLOSED` as a sticky
flag while draining native output, even without metrics observers. It does not
wait for native audio/decoder teardown or rely on the marker remaining in the
8 KiB error tail. The active session checks this flag before marking video
started or considering transport recovery, including startup stabilization.
Server cleanup retries without reopening the closed viewer. Server disconnect
already removes the corresponding accepted sharing member transactionally.

As a fallback, host `SNSHARE_PEER` status exposes bounded `stopReason` (0–5).
Reason 1 is an authenticated peer Disconnect, so the host ends only that
viewer's session and releases its slot. It retains a failed server cleanup for
retry, preserves other viewers, and checks requester readiness generation so
an obsolete sender cannot end a replacement route.

A transport-error Disconnect optionally carries its existing one-byte error
reason 2–5. These authenticated errors remain recoverable; they must not be
misclassified as deliberate departure. Empty Disconnect remains compatible
with older peers. Old telemetry without a reason defaults to 0 and never
triggers host-side terminal cleanup. Both updated peers are needed for error
reason propagation; local viewer-close cleanup works independently of it.
Closing the main shell still hides it to tray; Leave voice only leaves voice.

## Host awareness

An approved ordinary connection shows the computer name, approved view/control
permission, and Stop access. Group connections show an authenticated/started
viewer, View only, and Stop viewing. Invites alone do not generate joined
notifications. The alert disappears when the session ends and dismissing it
never disconnects. Alerts are deduplicated per session, bounded, and reset on
account changes; asynchronous OS delivery rechecks current ownership/liveness.

In-app notices work without OS permission. Host → Connection alerts → Enable
notifications explicitly asks OS permission. Connection events themselves do
not prompt. When permission is granted, local desktop notifications also work
while the main app is hidden. OS Do Not Disturb can suppress presentation;
Windows native notification verification requires an installed application.
No microphone or screen content enters a notification.

## Validation and remaining physical checks

Tests cover close while native teardown is running, startup close, retry after
server failure, session replacement, bounded live-pipe observation, isolated
sharing cleanup, reconnect preservation, notification permission/deduplication,
and stale asynchronous notification suppression. Authenticated UDP loopbacks
exercise deliberate close, error-reason propagation and surviving viewers.
No real microphone/screen or OS notifications are used by these tests.

Physical acceptance still requires two updated installed desktops: invite and
join, close the viewer, verify the host frees the slot and can invite again;
repeat with another viewer remaining active. Enable OS notifications, hide the
host shell, and connect with view/control permission. Verify one named alert,
correct Stop target, no reappearance on every poll, and no mic activity merely
from receiving the connection alert.

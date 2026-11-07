# Mobile network recovery — phase 4d

Source increment, 2026-11-05, Sanser 2.3.3. No release or phone installation.

## Trigger and user experience

Updated native engines observe the default network independently of the media
session: iOS uses `NWPathMonitor`; Android uses a default-network callback.
This is change notification rather than connection polling. The first snapshot
establishes a baseline; unchanged/duplicate notifications do not restart a
healthy stream. Android ignores changes to validation/signal quality alone.
Interface/address/route fingerprints remain native; Flutter receives only
availability, a change flag and a local sequence counter.

When a previously streaming session changes network, the controller shows
**Đang khôi phục kết nối** or **Mất mạng · đang chờ Wi-Fi hoặc dữ liệu di động**.
The accepted server session stays open and Cancel remains available. A usable
network snapshot is not proof that the Sanser host/server is reachable; the
connection still needs authenticated verification.

Recovery waits 700 ms after the last change, opens at most two media attempts
per episode, and has a 45-second total deadline including offline waiting.
Attempts are serialized. A stalled native teardown has a five-second limit and
ends recovery rather than opening another engine concurrently. Background apps
do not start a retry; the existing disconnect-on-background/document-picker
policy is preserved. Cancelling, signing out, disposal or deadline expiry
invalidates pending work and prevents automatic resurrection.

## Reconnect contract

The controller closes old media/Direct forwarding, announces a new
`native-ready` generation on the **same accepted session**, fetches current
credentials and renegotiates the route. The existing server endpoint advances
`requesterReadyAt` monotonically and verifies ownership/acceptance. No automatic
new session, invitation, trust approval or change of host occurs. Server
401/403/404/409 responses stop recovery rather than retrying revoked access.

- Auto gathers fresh candidates and can fall back to Relay after Direct fails.
- Forced Direct reports failure; it never silently becomes Relay.
- Explicit Relay skips UDP gathering.
- Group viewers retain view-only access and their existing viewer ID. The new
  first frame acknowledges the new ready generation for the sharing session.

Every media start has a local `engineAttempt` echoed on native media/status
events. Updated engines advertise `networkRecovery`; those events must match
the current attempt before they can update state. Old frame/stop messages cannot
make a new attempt appear healthy or terminate it. This supplements native
callback generations; it is not a wire authentication mechanism.

Input/file UI ownership is recreated when recovery begins. Queued MobileInput
commands recheck the original media revision at dispatch, and old error handlers
cannot reset the recovered engine. Held keys, local drafts and pending file
actions are not replayed. Reset is best-effort on the broken old path; normal
native teardown/host peer-timeout protections still apply. File transfers may
need a new explicit offer. System-audio mute is preserved, while voice/mic state
is cleared and late replies are ignored. No microphone is enabled by recovery.

Only a new native first-frame event completes recovery. Reconnecting resets
decode/audio/transport state; this is **not seamless route migration**. It does
not fix initial connection failures or automatically retry arbitrary remote
stops when no network change was observed. Old engines keep their prior behavior.

## Verification and remaining device checks

Twelve recovery tests use fake platform events and an in-memory server adapter:
baseline/debounce, stale events, fresh readiness, cancelled/late credentials,
serialized path changes, two-attempt/deadline bounds, failed native startup,
blocked teardown, forced Direct and Auto fallback, server revocation, background,
group view-only/first-frame acknowledgement, mute/mic replies and queued input.

All 102 Flutter tests pass and analyzer reports no issues. An isolated unsigned
iOS Release builds with the updated Swift monitor (23.6 MB Runner.app). All
Android Kotlin sources compile against the existing Android 36/Flutter/JVM 17
classpath; this is not an APK or Android runtime result. There is one existing
unnecessary-elvis warning in `Media.kt`. No signing settings were changed.

Before release, test real iPhone/Android Wi-Fi ↔ cellular handoff, same-interface
DHCP changes, VPN changes, offline/airplane mode, captive portal, screen lock,
background/document picker and manual cancellation during gather. Verify both
Direct and Relay, host revoke/end-session, group viewer membership, held keys,
audio mute and incomplete files. OS notifications can arrive late or miss a
particular same-interface route change; no universal handoff/latency guarantee
is claimed by unit tests.

Primary references: [Apple NWPathMonitor](https://developer.apple.com/documentation/network/nwpathmonitor/),
[Android network state](https://developer.android.com/develop/connectivity/network-ops/reading-network-state),
[Android callback ordering/race guidance](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback).

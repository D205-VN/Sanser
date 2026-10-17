# Part 3: bounded recovery of an accepted native session

Development on `Version-2.0.0` after 2.2.12; no new release/tag or installer.
The client can reopen its remote window with fresh authorization and candidate
gathering. This is a **controlled reconnect**, with a visible interruption,
not seamless migration of an existing media socket.

## Evidence required for automatic reconnect

The portable policy lives in `sanser-p2p::plan_route_recovery`; the desktop calls
it through a pure advisory command. Client lifecycle/session state holds the
attempt budget. No video/input data is sent through this IPC command.

| Evidence | Required condition |
| --- | --- |
| SNV2 peer lost | Exact terminal native peer-timeout marker, not a substring of another error |
| Relay lost | Explicit offline/connection/socket-receive failure; not authentication/cipher failures |
| Legacy control lost | A previously authenticated route has supplied no control responses for 15 seconds; current control-silence sample is fresh |
| Direct degrades after startup | At least 15 seconds since local engine launch, 3 nomination probes with median below 80 ms, 5 consecutive elevated live Wire RTT observations, and current RTT above max(80 ms, twice probe median, probe median + 40 ms) |
| High application RTT alone | Observe only; queues/peer processing have not been excluded |
| A path already slow at nomination | Observe only; a faster alternative has not been demonstrated |

Evidence older than three seconds is ignored. Live snapshots must belong to the
same accepted session and have started no earlier than the current local engine
launch. An ended/old session or previous engine's measurements cannot cause a
new window to be torn down. SNV2 currently supplies application RTT, not wire
RTT; automatic recovery of high RTT alone is consequently limited to the
legacy path's detailed wire estimates. Explicit SNV2 transport loss is supported
for every SNV2 host/client platform pairing and group viewers.

## Lifecycle

1. Recheck the server: the same session must still be accepted and authorized.
   A host closing/rejecting the session prevents recovery.
2. Stop only that session's client engine and relay. Session-scoped stop checks
   the process owner in Rust, so late cleanup cannot kill a different session.
   Status and relay failures are also matched to their owning session before
   stopping an engine or scheduling recovery.
3. Wait 1.5 seconds before the first automatic retry and 5 seconds before the
   second (actual launch also depends on UI polling).
4. Call native-ready again, fetch new credentials and create a new local P2P
   attempt. The existing server epoch makes the host rejoin fresh negotiation;
   candidate collection/scoring runs again instead of reusing stale endpoints.
5. Direct-only remains Direct-only. Relay-only remains Relay-only. Auto retries
   Direct and uses the existing authenticated relay fallback if needed.
6. The new client opens a fresh native window. It does not gain input or audio
   permission: group viewers remain view-only/video-only. A single group viewer's
   reconnect does not stop the shared host or other viewers.

The budget is **two automatic reconnects per accepted client session**, retained
across page navigation. A successful window launch does not reset it. A failed
recovery authorization/negotiation stops automatic retries and leaves manual
retry available. The pre-existing one-time blank-startup recovery is separate.
Only explicit manual Retry/Reconnect or starting another session resets budgets.

Disconnect/sign-out/session replacement cancels subsequent launch work. Cleanup
that already owns the same session can finish after page navigation and releases
the shared busy state. Automatic scheduling runs while Active session is mounted;
queued recovery resumes on returning to that page. Closing the main Windows
window to the tray does not itself unmount it. A normal remote-window close,
decoder crash, authentication failure or input ACK timeout does not reopen it.

The Reconnect button also lets the user recheck routes while a window is still
running, including a route already slow at nomination. Gathering failures and
cancelled/failed launches release their reserved P2P attempts.

## Verification and limits

Tests cover policy thresholds, stale/insufficient evidence, slow-at-nomination,
finite budgets, native/relay error classification, explicit user close,
navigation and Disconnect races, fresh credentials/generations, Direct-only
constraints, group permissions and session-scoped process cleanup. Synthetic
subprocess and loopback tests do not capture a screen or inject OS input.

No standby native socket is maintained and no better path is guaranteed. A
new candidate race can select the same route if it remains the best available
one. There is no automatic in-flight path switch, seamless rollback to the old
window, complete IPv6 media support, public UDP-relay deployment or assertion
that 200 ms becomes 10 ms. All four real desktop directions, changing routers,
VPN/NAT paths and group behavior still require physical-device testing.

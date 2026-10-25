# UAC transition: service enabled does not prove worker readiness

The user supplied the actual UAC prompt accompanying the frozen Sanser view.
This establishes a transition to Windows Secure Desktop in that reproduction.
The retained 2.2.21 reports around 17:19 also contain `capturePaused=1` while
authenticated control continues. They do not identify the broker's failing
stage. A game remaining unclickable after UAC closes is a separate elevated
input/target-app question, not evidence that the game uses anti-cheat blocking.

## REA and comparison

REA 6.5.0 was restored in `/private/tmp/sanser-uac-research-2026-10-25/rea` and
used for static artifact/Mach-O inspection of installed Parsec and AnyDesk. No
competitor was executed, attached to or captured. Initial Mach-O inspection
could not compile REA's helper in its normal cache; retry with temporary cache
and the required filesystem access completed. Evidence identifiers, digests and
pinned source hashes are in [the evidence record](uac-2026-10-25-evidence.json).

REA's Mac artifact evidence does not reveal proprietary Windows algorithms.
The following behavior is grounded in primary Windows documentation/source:

- [Parsec Windows service](https://support.parsec.app/hc/en-us/articles/32381199341716-Parsec-App-for-Windows) runs a SYSTEM service and restarts the interactive app with sufficient privileges for UAC/elevated input and capture.
- [RustDesk Windows code](https://github.com/rustdesk/rustdesk/blob/1297da6d186e58d9096be24072ae3998cdbbf8fe/src/platform/windows.cc) locates the target session, starts the interactive process with a suitable token, and uses `OpenInputDesktop` / `SetThreadDesktop`. Its [portable elevation documentation](https://rustdesk.com/docs/en/client/windows/windows-portable-elevation/) distinguishes installed service operation from an unelevated portable app.
- [Microsoft DuplicateOutput](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput) explicitly requires LOCAL_SYSTEM for the secure desktop. [SetThreadDesktop](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setthreaddesktop) can fail when the thread owns windows/hooks. Merely running a service is insufficient to establish either operation succeeded.

Sanser already uses a protected SYSTEM broker and per-connection interactive
workers. Its path/account/session checks, Default/Winlogon desktop allowlist,
unlocked-console restriction and no-network service model remain intact.
This patch does not support Windows sign-in/password desktops or change UAC
policy, desktop ACLs, caller authorization or the broker's binary installation.

## Confirmed timing defect and changes

The worker constructor allowed **3 seconds** to start, but the client waited
only **1 second** for the first response. A valid cold worker taking longer
than a second could lose its client before it was ready, be terminated during
cleanup, and repeat the same failure on retry. The first DXGI capture also
shared the short steady-state deadline. This is a demonstrable source defect;
it has not yet been proven to explain every user's screenshot.

1. A bounded, zero-payload **Probe → Ready** handshake now precedes every new
   worker connection. It carries no input and must pass normal authorization.
   Its 4-second ceiling covers the existing 3-second startup bound. There is no
   deliberate wait once the worker responds.
2. The first capture has a distinct 4-second cold-start ceiling on both sides.
   Steady responses retain their 1-second limit. A timed-out input transaction
   is never replayed by this layer.
3. Numeric health identifies pipe-open, server-verification, ready, request,
   response, frame and explicit-rejection stages. Explicit desktop/session/input
   rejection and capture HRESULT are retained instead of collapsing to empty
   unavailable responses. `ERROR_TIMEOUT` is recorded at the actual timeout.
4. Legacy authenticated pongs forward these counters to the Mac report. During
   capture pause, the Mac shows the **last helper failure** and code, retaining
   its historical meaning rather than presenting it as a current diagnosis.
   Old hosts without these fields do not invent a successful/failed helper.

`uacReady` counts established worker connections, not service installation.
`uacFrames/uacIdle` count accepted protected capture results. `uacFailures` is
cumulative; `uacStage/uacError/uacStatus` describe the last recorded failure.
For `uacStatus=8`, the error is a capture HRESULT bit pattern; otherwise it is a
Win32/status error. No screen/input contents, app names, paths or SIDs enter the
new numeric report. A failed readiness probe submits no click/key.

## Validation and limits

The BUILD_TESTING-only worker waits 1500 ms before connecting, deliberately
outlasting the old response deadline. The existing isolated Windows service
fixture must still accept actual medium-integrity input on its elevated Cancel
button, preserve account binding through update and reject unauthorized Probe,
Capture and Input callers. Production sidecar staging uses BUILD_TESTING=OFF,
so no synthetic worker delay or test entry point is packaged.

Protocol tests reject data-bearing probes; Mac tests distinguish missing health,
ready timeout, desktop denial and capture errors; Rust tests verify numeric
allowlisting. CI exercises worker readiness and elevated input on **Default**.
It does not automate an actual Windows consent prompt or prove Winlogon video
capture: test that on the affected Windows host after installing an updated
bundle. Use the new stage/code if the helper still fails. Also retest ordinary
apps and elevated launchers after dismissing UAC. These source changes are not
in the published 2.2.21 installer.

# Windows UAC support

The normal Windows host runs as the signed-in user. DXGI loses access when an
installer switches to the UAC secure desktop. Retrying duplication preserves the
session but does not grant access to that desktop. This change adds an optional
LocalSystem service for that interval, shared by the legacy and SNV2 hosts.

## Enabling it

In a build containing the helper, open **Host → Windows administrator prompts →
Enable UAC support** on Windows. Stop any current host/group session first.
Approve Windows' administrator prompt locally. The first setup prompt cannot be
controlled remotely before the helper is installed.

Setup copies the service and host into the protected `Program Files/SanserUac`
directory and registers `SanserUac` with automatic start. Subsequent host sessions
use the protected host if its capabilities/version match the app. A version
change requires **Update UAC support**. The per-user Tauri application continues
running without elevation. **Disable UAC support** stops and unregisters the
service, with administrator approval; protected binary copies remain on disk.
Installer and portable packaging both include the helper. Installing/updating
Sanser alone does not silently install or enable this service.

## Data flow and scope

1. Normal capture and input use the existing in-process DXGI/GPU/input paths.
2. If DXGI becomes unavailable, the protected host requests local UAC capture.
3. The Session 0 broker launches a fixed, GUI-free LocalSystem worker in the
   caller's active console session. Windows prohibits inheriting handles across
   sessions; a random, SYSTEM-only named pipe carries worker IPC instead.
4. The worker binds its capture/input thread to the `Winlogon` input desktop.
   It captures BGRA with DXGI. Only this temporary interval uses CPU readback
   over local IPC; the host uploads to its existing GPU device for encoding.
5. Network authentication, encryption, peer/view-only permissions, encoding and
   audio remain in the normal host. The service has no network listener and
   accepts no commands, executable paths, credentials or session tokens.
6. The host retries normal DXGI every 250 ms. On recovery it changes media
   generation, requests an IDR through the existing host loop and resumes the
   normal pipeline. The network session is not restarted.

Input uses the helper while it is actively serving UAC capture, or when normal
`SendInput` is blocked by an elevated installer on the unlocked Default desktop.
Both cases use the same authenticated, protected host and user/session checks.
Applied keys/buttons are tracked by the worker and released on graceful pipe
disconnect so a viewer cannot leave modifiers held inside the secure desktop.
A confirmed
`Unavailable` response can return an input event to the normal desktop after UAC
closes. An input transaction whose outcome is unknown is not blindly replayed.

This feature deliberately excludes locked sessions, sign-in/password screens,
RDP and other users' sessions. The broker verifies that the caller is the fixed
protected host executable, belongs to the console user, and that the console is
active and unlocked. It rechecks that state on each operation. Enabling this
privileged feature authorizes that user's trusted Sanser host to interact with
administrator prompts during accepted remote sessions; it does not auto-approve
UAC prompts or change Windows UAC policy.

## Lifetime and validation

The public pipe rejects remote clients, has an explicit local ACL and authenticates
the client's PID, executable path and user SID. The host checks the server image
and running SCM service PID without requiring permission to read a SYSTEM token.
The worker executable and host are installed with protected administrator/SYSTEM
ownership and user read/execute permissions. Setup rejects reparse-point targets
and replaces files rather than overwriting a possible hard link. The worker has
a kill-on-job-close lifetime; service stop, host exit, broken IPC or transaction
timeout cannot leave an independent capture worker running. Concurrent workers
and message/frame sizes are bounded. No frame content is written to logs.

`SNUAC_CAPTURE active=1 source=system-helper` and
`SNUAC_CAPTURE active=0 source=desktop` mark host transitions. A missing/disabled
helper keeps the existing capture-paused behavior and the session alive.

Automated checks cover bounded IPC parsing, malformed payload rejection, explicit
UI approval/cancellation, packaging, and on Windows CI actual service
installation/start/stop, protected ACLs and rejection of an unauthorized test
executable. These do **not** prove UAC video/input on physical GPU hardware.

Manual acceptance still requires a Windows/Mac pair: establish ordinary and
group sessions, launch a benign installer, verify that its real UAC prompt is
visible and mouse/keyboard work, cancel/accept it, and confirm moving desktop
content resumes without reconnecting. Also test disabling the service, viewer
disconnect while UAC is open, repeated UAC transitions, and lock/logoff rejection.
Include Windows scaling at 125%/150%: the worker sets the same DPI awareness as
the normal host before DXGI queries, so logical dimensions do not conflict with
physical duplication surfaces.
Do not enter or record administrator passwords in test artifacts.

## References

- [Microsoft: DuplicateOutput](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput): secure desktop access requires LocalSystem.
- [Microsoft: desktop duplication](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api): recreate duplication after access loss.
- [Microsoft: CreateProcessAsUser](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw): session/desktop selection and cross-session handle restrictions.
- [Microsoft: interactive services](https://learn.microsoft.com/en-us/windows/win32/services/interactive-services): services use a separate worker in the interactive session rather than an interactive Session 0 service.
- [Microsoft: GetDesc](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgioutput-getdesc): output dimensions are virtualized unless the process is DPI aware.

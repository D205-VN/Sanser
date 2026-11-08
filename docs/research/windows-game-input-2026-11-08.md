# Elevated game input and a broker that never became ready

The user's most recent retained Direct report contains 1,247 direct OS input
acceptances, zero privileged acceptances and 5,544 OS input rejections. Source
integrity is 8,192 (medium), foreground target integrity 12,288 (high). Broker
health reports 61 failures, no ready connections, stage 3 (readiness), Win32
232 (pipe closing). The user confirms input resumes only after the game exits.
The report alone does not distinguish admission rejection from worker startup
failure; both previously closed the pipe without a detailed reply.

Microsoft documents [SendInput's integrity restriction](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput).
It also notes that a zero return/GetLastError cannot identify UIPI by itself.
Here the integrity measurements and failed privileged channel provide the
additional evidence; this is not an attribution to network jitter or proof
of a game-specific anti-cheat block.

The launcher previously checked the helper, then did another best-effort lookup
while selecting the host. A transient failure in that second lookup could
select the ordinary user-writable host, which the SYSTEM broker correctly
rejects. The patch resolves the enabled protected host once, propagates
lookup failures and refuses a silent fallback. Explicitly disabled/unavailable
support retains the ordinary host; view-only sessions do not require an input
readiness probe.

An enabled input session now runs the selected protected host's fixed
`--uac-ready-json` probe before launching its media session. It opens no
capture, sends no keys/clicks and returns only readiness plus bounded numeric
failure fields. The Host readiness indicator also checks this channel, rather
than equating a running service or capability probe with working injection.
These checks run off the GUI thread.

The broker consumes the fixed initial probe before admission/worker creation.
It returns a fixed rejection with a numeric check/error for admission or
worker-start failures. Executable path, account binding, same-user session,
unlocked-console, service identity and protected-file restrictions are kept.
Rejections include no SID, token, path or frame data. A bounded drain lets the
client consume the reply without an unbounded flush. Failed readiness now
propagates its error to input health instead of leaving error/status zero.
An ambiguous timed-out click/key is never replayed.

Regression tests cover enabled-helper fallback, lookup failure, stale/missing
paths, disabled support, real readiness versus running/current status and
privacy of errors. Windows CI additionally probes the actual protected host
and broker, checks unauthorized executable rejection, and retains the elevated
Cancel fixture. Headless/unlocked-console rejection must be explicitly marked
as untested physical input, not a successful game click.

These changes address a verified launch fallback and missing readiness checks.
They do not yet prove the precise rejection on the user's Windows installation
or certify the reported game. Retest with the updated Windows host/helper and
desktop shell; retain numeric readiness output if admission still fails.
Audio and video scheduling are unchanged.

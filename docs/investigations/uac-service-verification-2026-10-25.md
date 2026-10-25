# Windows input stalls after an elevated application opens

## Evidence from 2.2.22

Two locally retained, numeric connection reports from October 25 identify a
failure before the helper readiness handshake. No screen, input content, account
identifier or remote endpoint is included here.

| Session start (UTC) | Input allowed | Source / target integrity | Direct inputs | Privileged inputs | Rejected inputs | Helper stage / error | Ready connections |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 12:06:40 | 1 | 8192 / 12288 | 50 | 0 | 4685 | 2 / 5 | 0 |
| 12:08:15 | 1 | 8192 / 12288 | 20 | 0 | 3831 | 2 / 5 | 0 |

The later session contains input acknowledgments with `applied=0`, while sampled
RTT is approximately 9–14 ms. Permission to control the host is enabled, but the
privileged execution path has not accepted any event. In the earlier session
there are also network latency spikes; they do not explain the stage-2 failure.

Stage 2 is endpoint verification, before Probe/Ready. Version 2.2.22 converted
**every** failure in that check to Win32 error 5, so these reports cannot identify
which verification subcheck failed. They do establish that the prior readiness
timeout change cannot fix this particular failure.

When the foreground process has higher integrity, input deliberately uses the
helper. Consequently an unavailable helper also prevents a click on the taskbar
from switching away from that foreground process.

## Correction

Remove the client's dependency on opening and querying the SYSTEM service
process. Instead, verify the connected pipe using read-only Service Control
Manager queries:

1. Read the connected pipe's server PID.
2. Require the registered SanserUac service to be running with that exact PID.
3. Require an own-process service, configured as LocalSystem.
4. Require the exact quoted service command in the protected SanserUac Program
   Files directory, including `--service`.

This retains endpoint identity checks without granting a standard user process
access to SYSTEM process objects. Host authorization, allowed-user binding,
desktop policy, readiness handshake and input execution checks remain enforced.

`uacCheck` now distinguishes pipe PID (1), SCM access (2), service access (3),
running PID (4), configuration query (5), service account/type (6), and installation
command (7). Failures retain their real Win32 code instead of always reporting 5.
The control response, Mac client and sanitized connection report carry this
numeric field. The Mac warning describes the last recorded failure, which may
precede a later successful attempt.

## Regression test

The former synthetic input fixture lowered integrity and stripped privileges,
but retained an enabled Administrators group SID. The corrected fixture also
marks that SID deny-only. In an isolated Windows CI runner, it temporarily denies
that user's PROCESS_QUERY_LIMITED_INFORMATION access to the synthetic SYSTEM
service process. The original process DACL is restored on fixture exit; this test
code is excluded from production builds.

Hosted CI may grant its interactive desktop through Administrators only. A child
with that SID deny-only exited with `0xC0000142` before reaching its test entry
point on the first run. The fixture now temporarily grants its own user access to
the CI window station/desktop as required by CreateProcessAsUser, restoring both
DACLs on exit. This is confined to the isolated fixture; production desktop ACLs
are not changed.

The protected medium-integrity host must demonstrate both the deny-only SID and
failure of the old OpenProcess dependency, then deliver a real click to the
fixture's elevated Cancel button through the authorized helper. A separate
user-owned pipe must fail the service PID check. The existing IPC denial,
installer update/account preservation and protected installation ACL tests still
run.

This is a test on an elevated **Default** desktop window, not an actual Winlogon
UAC prompt or the user's game. Successful CI validates this permission regression;
the exact game and Secure Desktop transition still require a two-machine test.

This patch is subsequent to the published 2.2.22 installer.

## References

- [Restricted tokens](https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens)
- [Service security and access rights](https://learn.microsoft.com/en-us/windows/win32/services/service-security-and-access-rights)
- [QueryServiceStatusEx](https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nf-winsvc-queryservicestatusex)

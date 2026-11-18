# Session recording and "Report a problem"

## Session recording (viewer side)

The viewer can press **Record** on the Active session page during a 1:1
session or while watching a shared screen. The file is written on the viewer
computer only, in `Downloads/Sanser/Recordings/sanser-recording-YYYYMMDD-HHMMSSZ.mp4`
(UTC time stamp; a numeric suffix avoids overwriting).

### What is recorded

- **Video:** the H.264 access units the viewer already received, muxed
  without re-encoding into a fragmented MP4 (`native/common/mp4_recorder.h`).
- **Audio:** system audio, only if the viewer was receiving it when recording
  started. Both viewers already hold 48 kHz stereo PCM (decoded Opus or PCM
  from the host), so it is encoded once more with the shared Opus encoder and
  stored as an **Opus track inside the same MP4** (`Opus` sample entry +
  `dOps`). We chose one file over a sidecar so a recording is a single thing to
  share. Opus-in-MP4 plays in VLC, ffmpeg/ffplay, Chrome, Edge/Windows Media
  Player (Windows 10+); see "To check" for QuickTime.
- **HEVC:** not recorded. SNV2 sessions (the only path with the stdin control
  channel) are H.264-only by design. The legacy Mac-to-Mac path may use HEVC
  and has no control channel, so the page shows "Recording needs an SNV2
  (H.264) connection" instead of a Record button. The tap also stops with
  `unsupported-codec` if it ever sees HEVC parameter sets.

### Robustness

- A file never starts on a non-IDR picture. Starting a recording requests a
  keyframe immediately and again at most once a second while waiting.
- A missing frame id (lost picture, queue overflow, decoder recovery) makes
  the recorder drop pictures until the next IDR; the file shows a short freeze
  instead of corrupted references.
- New SPS/PPS or resolution at an IDR closes the file and continues in
  `…-part2.mp4` (one sample description per file keeps every player happy).
- Fragments (`moof`/`mdat`) are flushed at each IDR or every ≤2 s, so a crash
  or full disk leaves a playable file. On a clean stop the header duration is
  patched.
- Limits: 2 hours or 3.8 GB (FAT32-safe), whichever comes first; the viewer
  sees why the recording stopped. Disk writes run on a worker thread with a
  64 MB queue; if the disk falls behind, pictures are dropped until the next
  IDR rather than blocking decode.

### Consent

1. Viewer → `POST /api/v2/sessions/{id}/recording {action: "request"}`.
2. The host app polls this state for its accepted 1:1 and sharing sessions
   every 2 s and answers `allow`/`deny` from **Host → Allow viewers to
   record** (default on). While allowed, every page of the host app shows
   "*Device* is recording your screen on their computer" with a **Stop their
   recording** button (sends `deny`).
3. The viewer starts the native recorder only after `allowed`. No answer in
   10 s (e.g. an older host app) means no recording.
4. While recording, the viewer re-sends `request` every 10 s; a `denied`
   answer stops the recording. State is in server memory with a 30 s TTL, so a
   crashed viewer clears the host notice within 30 s.

The server relays state only; it never sees media or file names. The display
name comes from the viewer's device record. Consent is enforced by the
official viewer app: someone running a modified client or a screen recorder
can always capture what they see.

### Plumbing

- Tauri → native: `record\tstart\t<maxBytes>\t<maxSeconds>\t<path>` /
  `record\tstop` on the existing stdin control pipe.
- Native → Tauri: private `SNUSER_RECORD\t…` lines (capable, waiting,
  recording, stopped with reason/bytes/duration/parts, error). They are
  filtered out of logs and connection reports and never contain the path.
- Code: `native/desktop/recording_tap.h` (shared by
  `native/platforms/macos/backend.mm` and `native/platforms/windows/backend.cpp`),
  `apps/desktop/src-tauri/src/engine_user.rs` (status), `commands.rs`
  (`start/stop_session_recording`, `session_recording_status`,
  `open_recordings_folder`), `apps/server/src/recording_consent.rs`,
  `apps/desktop/src/stores/recording.ts`, `SessionRecording.svelte`,
  `HostRecordingNotice.svelte`.

## Report a problem

Settings → Diagnostics → **Report a problem** writes
`Downloads/Sanser/Reports/sanser-report-<UTC>Z.zip` and reveals it in
Finder/Explorer. It contains `README.txt`, `system.json` (app version, OS,
OS version, architecture, runtime capabilities), `diagnostics.json` (event
buffer plus connection reports) and up to five `connection-reports/*.json`.

Before writing, every key and string is redacted
(`apps/desktop/src-tauri/src/problem_report.rs`): secret-like keys
(password, token, credential, secret, authorization, cookie, signature,
email, …) and name keys (`name`, `deviceName`, `displayName`, …) are replaced;
free text loses e-mail addresses, IPv4/IPv6 literals (ports kept), URL
credentials, secret query values, IP hosts, `Bearer` values, `key=value`
secrets, long opaque tokens/JWTs, the home directory (→ `~`) and the account
user name. UUIDs are kept so reports can be correlated.

**Open GitHub issue** opens `https://github.com/D205-VN/Sanser/issues/new`
with a template body containing only the app version and OS family; the shell
re-validates the URL (exact host/path, only `title`/`body`/`labels`, no
personal data) before handing it to the browser. Nothing is uploaded; the user
attaches the zip manually after reviewing it.

## Tests

- `native/common/mp4_recorder_test.cpp` (CTest `sanser-mp4-recorder-test`):
  real fixture stream + real Opus; IDR wait, lost picture, part split, size and
  duration limits, write error, threaded stop; every output is decoded with
  `ffmpeg -v error` when ffmpeg is installed.
- `native/desktop/recording_tap_test.cpp`: command parsing, status lines, HEVC
  rejection, no path echo.
- `cargo test -p sanser-desktop`: redaction, zip format/CRC, issue URL
  validation, recording status parsing.
- `cargo test -p sanser-server`: consent state machine.
- `npm test` in `apps/desktop`: issue URL builder, consent flow, host poll.

## Still to check on real machines

- [ ] Windows viewer: build `sanser-client-windows` with the new tap
      (`_wfopen_s` path, non-ASCII user folders) and record a session.
- [ ] macOS viewer recording a Windows host and a shared screen, with and
      without system audio; play the file in QuickTime, VLC and a browser.
- [ ] QuickTime Player / Photos support for Opus-in-MP4 on macOS 12–15. If it
      does not play the audio, consider AAC via AudioToolbox / Media
      Foundation as a follow-up.
- [ ] Resolution change mid-session (mobile resolution policy, window resize)
      produces `-part2.mp4` and both parts play.
- [ ] Packet loss/relay recovery: file has freezes, no green/grey corruption.
- [ ] A/V sync over a 30+ minute recording (video uses local receive time,
      audio its playout time).
- [ ] Long recording to the 2 h / 3.8 GB limit; disk-full behaviour.
- [ ] Host notice with two viewers recording in a sharing group; old host
      app (no answer) → viewer shows "host did not confirm".
- [ ] Server deployed with more than one instance would need shared consent
      state (currently in-process memory).
- [ ] "Report a problem" reveals the zip in Finder/Explorer and the GitHub
      issue page opens in the default browser on both OSes.

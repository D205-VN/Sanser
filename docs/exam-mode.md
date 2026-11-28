# Exam mode

Sidebar → **Exam**. One computer supervises; each student joins from their own
Sanser account and computer with a 6-character code.

## Supervisor

- **Start an exam**: name, longest duration (1–8 h, then the room ends by
  itself), allowed apps (presets carry both the macOS bundle id and the Windows
  `.exe`, or add any app), and **Block AI**.
- The room shows the code, a grid of student screens (refreshed every second,
  each picture arriving only when that screen changed), the app each student
  has in front, a violation count and a violation feed.
- Clicking a student opens a large live view: that computer sends about four
  sharp pictures a second (1280 px, JPEG quality 45, ≈ 80 KB, still only when
  the screen changed) for the next 90 s, renewed while the view stays open,
  and the supervisor polls four times a second. One student at a time; closing
  the view takes them back to the grid pace. It is fast pictures, not video:
  enough to read code, but scrolling looks jerky.
- **Pause this computer** covers the student's screen with Sanser (full screen,
  always on top) until **Let them continue**. **Remove from exam** unlocks that
  computer; it cannot rejoin the same room. **End exam** unlocks everyone.

## Student computer

While the room is open and the student is not removed:

- A guard script (`exam_guard/guard_macos.js` through `osascript`,
  `exam_guard/guard_windows.ps1` through PowerShell) checks the front app every
  350 ms. Any app other than the allowed ones and Sanser is hidden/minimized,
  the allowed app is brought back, and a `left-app` violation is reported.
  Windows taskbar/Start clicks are undone silently.
- With Block AI, AI apps (ChatGPT, Claude, Gemini, Copilot, Perplexity,
  DeepSeek, Grok, LM Studio, Ollama, Cursor…) are force-closed (`ai-app`), and
  window titles of AI websites are reported (`ai-site`) while Sanser jumps in
  front. On macOS reading window titles needs Screen Recording permission,
  which screenshots need anyway.
- Sanser replaces its whole window with the exam screen, leaves any remote
  session, takes Host offline, and refuses to quit (Cmd+Q, Dock, tray, window
  close) and to sign out. The seat is stored locally, so after a crash or
  restart the lock comes back at once.
- Heartbeat every 5 s (2 s while watched) with the front app and violations.
  The answer carries pause/end/remove.
- Screens go on their own (`POST /api/v2/exam-members/{member}/screen`, newer
  servers announce it with `liveScreenSeconds`/`liveScreenMs`): one capture a
  second, about 640 px at JPEG quality 35 (≈ 20–30 KB); every 250 ms at
  1280 px and quality 45 while watched. Each picture's answer carries the
  pace, so opening a student takes effect within a second, not at the next
  heartbeat. Each is
  reduced to a 64×36 grayscale fingerprint and sent only when a cell moved by
  more than 10, or after 10 s unchanged. Windows keeps one hidden PowerShell
  for captures instead of starting one per picture; it ends with the exam or
  with Sanser. Against an older server the picture rides in the heartbeat
  every 10 s (2 s while watched) as before.
- Offline: the lock stays and violations are kept and sent later. After 9 h
  without the server the seat is released.

## Emergency unlock

Each room has an 8-character emergency code. The supervisor reveals it under
the room code (hidden by default, so a projector does not show it) and reads
it only to a stuck student. The student types it on the lock screen under
**Emergency unlock**: the server checks it, removes that student and records
an `emergency-unlock` entry. When the server is out of reach the computer
checks the code against a salted PBKDF2-HMAC-SHA256 hash (100 000 rounds) it
received when joining and kept with its seat, so it works offline and after a
restart. Rooms created before 2.3.37 have no code.

The guard scripts also stop by themselves when Sanser is closed or killed, so
apps never stay hidden without Sanser.

## Server

`/api/v2/exams…` (supervisor) and `/api/v2/exam-members…` (student); see
`apps/server/openapi.json`. Screenshots and watch requests live only in server
memory (`exam_live.rs`) and are dropped after 3 minutes or when the room ends.
A silence over 30 s is written as a `disconnected` violation when the student
returns. Exam heartbeats skip the per-address limit (a class shares one school
address) and use a per-seat limit of 40/min instead; screens 90/min (320/min
for the watched student). The supervisor's screen polling skips it too and
has a per-account limit of 400/min. A seat
may send screens for 30 s after each heartbeat without a database lookup, and
each screen gets a sequence number, so the supervisor polls every second with
`?after=<seq>` and gets only pictures it has not seen. Ended rooms are deleted
after 30 days.

## Limits (what it cannot stop)

It is an app-level lockdown, not a managed (MDM/kiosk) device. A student with
an administrator account can kill Sanser from another account or a second
device; the supervisor then sees **Disconnected** and a `disconnected`
violation. AI built into an allowed app (e.g. Copilot inside Word) and a phone
next to the computer are out of reach.

# Exam mode

Sidebar → **Exam**. One computer supervises; each student joins from their own
Sanser account and computer with a 6-character code.

## Supervisor

- **Start an exam**: name, longest duration (1–8 h, then the room ends by
  itself), allowed apps (presets carry both the macOS bundle id and the Windows
  `.exe`, or add any app), and **Block AI**.
- The room shows the code, a grid of student screens (every 10 s), the app each
  student has in front, a violation count and a violation feed.
- Clicking a student opens a large view and asks that computer for a picture
  every 2 s for the next 90 s (renewed while the view stays open).
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
- Heartbeat every 5 s (2 s while watched) with the front app, violations and a
  JPEG screenshot (≤ 180 KB). The answer carries pause/end/remove.
- Offline: the lock stays and violations are kept and sent later. After 9 h
  without the server the seat is released.

## Server

`/api/v2/exams…` (supervisor) and `/api/v2/exam-members…` (student); see
`apps/server/openapi.json`. Screenshots and watch requests live only in server
memory (`exam_live.rs`) and are dropped after 3 minutes or when the room ends.
A silence over 30 s is written as a `disconnected` violation when the student
returns. Exam heartbeats skip the per-address limit (a class shares one school
address) and use a per-seat limit of 40/min instead. Ended rooms are deleted
after 30 days.

## Limits (what it cannot stop)

It is an app-level lockdown, not a managed (MDM/kiosk) device. A student with
an administrator account can kill Sanser from another account or a second
device; the supervisor then sees **Disconnected** and a `disconnected`
violation. AI built into an allowed app (e.g. Copilot inside Word) and a phone
next to the computer are out of reach.

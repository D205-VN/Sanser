# Multi-monitor selection and host privacy screen

Source change on branch `claude/display-privacy-a7ec45`. Nothing here was run on
real Windows hardware or on a two-computer session. No latency was measured.

## Multi-monitor selection (SNV2 only)

- The host advertises its displays with an optional authenticated Keepalive
  extension `SDL1` (selected index, up to 16 × width/height/primary). The viewer
  asks for a display with `SDS1`. Older peers ignore both, so they stay on one display.
- The host checks every request against its own list. Then it re-targets capture
  and requests a fresh IDR.
  - Windows: `DesktopDuplicator::initialize(adapter, output)` runs again. The
    existing generation path refreshes pointer bounds, rebuilds the encoder when
    the adapter changes and forces a keyframe. If the switch fails, the old output stays.
  - macOS: `SCStream updateContentFilter` plus `updateConfiguration` at the new
    aspect ratio, `encoder.resize`, then `MacInputInjector::setBounds` and a keyframe.
- Shell path: the engine prints `SNUSER_DISPLAY` on stdout. Tauri exposes the
  `session_displays` and `select_session_display` commands, and the latter only
  accepts an index the host advertised. `SessionDisplays.svelte` on Active
  session shows a picker when there are two or more displays.
- Legacy (Mac viewer → Windows host) and multi-viewer shares stay single-display.
  The UI says so for legacy sessions.
- The display list is read once when the session starts. Hot-plugging a monitor
  needs a reconnect.

## Host privacy screen

- Settings: `preferences.host.privacyScreen` and `privacyBlockInput`, both off
  by default. They appear on the Host page and in Settings → Host.
- `host.ts` passes these local preferences to the host engine as
  `--privacy-screen` / `--privacy-block-input`. Only host engines get them, and
  never in a multi-viewer share. No SNV2 message can turn them on, so the viewer
  cannot enable either one.
- Windows (SNV2 and legacy host): a topmost, click-through black window covers
  the virtual screen, with `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`
  (Windows 10 2004+). If that call fails, the overlay is never shown. The SNV2
  host shows it once the viewer is authenticated. The legacy host shows it for
  the whole lifetime of the engine process.
- Blocking local input (Windows): a new UAC helper operation `Block` calls
  `BlockInput` on the worker thread that also injects remote input. While the
  block is on, all remote input goes through that worker. It is released when
  the worker exits: engine exit or crash, pipe loss, or the owner process dying.
  Without the helper the toggle is disabled and the UI says why.
- macOS: borderless black windows at screen-saver level that ignore mouse
  events. They are excluded from ScreenCaptureKit by window ID through
  `SCContentFilter`. Blocking local input is not offered on macOS.
- Local escape: Ctrl+Alt+Shift+P on Windows, Control+Option+Command+P on macOS.
  Either one hides the overlay for the rest of the session. While local input is
  blocked, press Ctrl+Alt+Del first: Windows always cancels BlockInput then.
- Ending the session or a crash of the engine removes the overlay, because the
  windows belong to the engine process.
- Diagnostics write `SNPRIVACY …` and `SNUAC_BLOCK_INPUT …` to stderr.

## Tests

- `sanser-display-select-test`: codec, retry, and an authenticated UDP loopback switch.
- `sanser-privacy-screen-policy-test`.
- UAC `protocol_test`, extended with `Block`.
- Rust tests: display parsing and privacy launch arguments.
- Vitest: `SessionDisplays`, `HostPrivacySettings`, preferences, host launch.

## Needs real hardware validation

- Windows build itself. The Windows code was not compiled in this macOS workspace.
- WDA exclusion with Desktop Duplication.
- Click-through for injected input.
- BlockInput through the helper, and recovery with Ctrl+Alt+Del.
- Switching between outputs on different GPUs.
- Portrait and mixed-DPI displays.
- macOS: the alpha-0 overlay windows appearing in SCShareableContent.
- macOS: the Carbon hotkey firing in the host loop.
- macOS: `updateContentFilter` across displays with different scale factors.

# Sanser Mobile 2.3.33

- **Type straight into the computer:** "Bàn phím" now opens the phone's own
  keyboard over the picture, with no text box. Tap a field on the computer
  and type: letters appear there as you type, Backspace deletes on the
  computer (also text that was already in the field), and the keyboard's
  return key presses Enter. A slim row above the keyboard has Hide, Esc,
  Tab, ⌫, ↵ and the arrows, and "Soạn rồi gửi" for composing a longer text
  first. The keyboard covers the bottom of the picture instead of shrinking
  it; pinch to zoom shows what is underneath.
- **Clear message when the computer stops:** when the computer stops sharing
  or ends the session, the home screen shows a neutral note ("Máy tính đã
  dừng chia sẻ màn hình." / "Máy tính đã kết thúc phiên điều khiển.")
  instead of "Engine native đã dừng (1).". Real connection problems are
  explained in plain words.
- **Easier chat:** Chat is the first button in the session controls, and the
  chat sheet is taller with larger text, your messages on the right in a
  blue-violet bubble and others on the left under the sender's name.
- **Smooth mode (Settings, off by default):** holds one or two frames and
  shows them at an even pace, for watching video over uneven Wi‑Fi, at about
  20–35 ms of extra delay. It cannot help when the computer sends few frames
  (a congested Wi‑Fi link); lowering "Bitrate video tối đa" helps there.
- **iPhone diagnostics:** the app logs video frame timing (arrival, decode,
  display) every two seconds, alongside the audio events from 2.3.32. No
  picture or sound content is logged.

Checks: mobile Flutter tests (171) and analysis pass; iOS release build
installed on an iPhone XS Max; Android Kotlin compiles. Smooth mode and the
direct keyboard have only been briefly tried on one iPhone.

Mobile version: 2.3.33, build 20341.

Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class NativeInput {
  [DllImport("user32.dll")]
  public static extern bool SetCursorPos(int X, int Y);

  [DllImport("user32.dll")]
  public static extern void mouse_event(uint dwFlags, uint dx, uint dy, int dwData, UIntPtr dwExtraInfo);

  [DllImport("user32.dll")]
  public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
}
"@

$MOUSEEVENTF_LEFTDOWN = 0x0002
$MOUSEEVENTF_LEFTUP = 0x0004
$MOUSEEVENTF_RIGHTDOWN = 0x0008
$MOUSEEVENTF_RIGHTUP = 0x0010
$MOUSEEVENTF_MIDDLEDOWN = 0x0020
$MOUSEEVENTF_MIDDLEUP = 0x0040
$MOUSEEVENTF_WHEEL = 0x0800
$KEYEVENTF_KEYUP = 0x0002
$VK_SHIFT = 0x10
$VK_CONTROL = 0x11
$VK_MENU = 0x12
$VK_LWIN = 0x5B

function Get-ButtonFlags($button, $down) {
  if ($button -eq "right") {
    if ($down) { return $MOUSEEVENTF_RIGHTDOWN }
    return $MOUSEEVENTF_RIGHTUP
  }
  if ($button -eq "middle") {
    if ($down) { return $MOUSEEVENTF_MIDDLEDOWN }
    return $MOUSEEVENTF_MIDDLEUP
  }
  if ($down) { return $MOUSEEVENTF_LEFTDOWN }
  return $MOUSEEVENTF_LEFTUP
}

function Get-VirtualKey($key, $code) {
  $name = if ($key) { [string]$key } else { [string]$code }
  switch ($name) {
    "Backspace" { return 0x08 }
    "Tab" { return 0x09 }
    "Enter" { return 0x0D }
    "Shift" { return 0x10 }
    "Control" { return 0x11 }
    "Alt" { return 0x12 }
    "Escape" { return 0x1B }
    " " { return 0x20 }
    "Space" { return 0x20 }
    "PageUp" { return 0x21 }
    "PageDown" { return 0x22 }
    "End" { return 0x23 }
    "Home" { return 0x24 }
    "ArrowLeft" { return 0x25 }
    "ArrowUp" { return 0x26 }
    "ArrowRight" { return 0x27 }
    "ArrowDown" { return 0x28 }
    "Delete" { return 0x2E }
  }
  if ($code -match "^Digit([0-9])$") { return [byte][char]$Matches[1] }
  if ($code -match "^Key([A-Z])$") { return [byte][char]$Matches[1] }
  if ($name.Length -eq 1) { return [byte][char]$name.ToUpperInvariant() }
  return 0
}

function Is-ModifierKey($vk) {
  return $vk -eq $VK_SHIFT -or $vk -eq $VK_CONTROL -or $vk -eq $VK_MENU -or $vk -eq $VK_LWIN
}

function Send-Key($vk, $up) {
  $flags = 0
  if ($up) { $flags = $KEYEVENTF_KEYUP }
  [NativeInput]::keybd_event([byte]$vk, 0, [uint32]$flags, [UIntPtr]::Zero)
}

function Send-Modifiers($payload, $up) {
  if ($payload.ctrl) { Send-Key $VK_CONTROL $up }
  if ($payload.alt) { Send-Key $VK_MENU $up }
  if ($payload.shift) { Send-Key $VK_SHIFT $up }
  if ($payload.meta) { Send-Key $VK_LWIN $up }
}

function Send-KeyStroke($payload, $down) {
  $vk = Get-VirtualKey $payload.key $payload.code
  if ($vk -eq 0) { return }
  if (Is-ModifierKey $vk) {
    Send-Key $vk (-not $down)
    return
  }
  if ($down) {
    Send-Modifiers $payload $false
    Send-Key $vk $false
  } else {
    Send-Key $vk $true
    Send-Modifiers $payload $true
  }
}

while ($line = [Console]::In.ReadLine()) {
  try {
    $payload = $line | ConvertFrom-Json
    $type = [string]$payload.type

    if ($type.StartsWith("pointer")) {
      [NativeInput]::SetCursorPos([int]$payload.screenX, [int]$payload.screenY) | Out-Null
      if ($type -eq "pointer-down" -or $type -eq "pointer-up") {
        $flags = Get-ButtonFlags ([string]$payload.buttonName) ($type -eq "pointer-down")
        [NativeInput]::mouse_event([uint32]$flags, 0, 0, 0, [UIntPtr]::Zero)
      }
      continue
    }

    if ($type -eq "wheel") {
      $delta = [int](-1 * [double]$payload.dy)
      [NativeInput]::mouse_event([uint32]$MOUSEEVENTF_WHEEL, 0, 0, $delta, [UIntPtr]::Zero)
      continue
    }

    if ($type -eq "key-down" -or $type -eq "key-up") {
      Send-KeyStroke $payload ($type -eq "key-down")
    }
  } catch {
    # Keep the worker alive even if one event is malformed.
  }
}

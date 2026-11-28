# Sanser exam guard for Windows. Run by exam_guard.rs with its settings in
# SANSER_EXAM_GUARD. Prints one JSON object per line on stdout.
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class SanserGuard {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
  public static string Title(IntPtr window) {
    var text = new StringBuilder(512);
    GetWindowText(window, text, text.Capacity);
    return text.ToString();
  }
  public static void Bring(IntPtr window) {
    if (IsIconic(window)) ShowWindow(window, 9);
    // A tap of Alt lets this process move the foreground.
    keybd_event(0x12, 0, 0, UIntPtr.Zero);
    keybd_event(0x12, 0, 2, UIntPtr.Zero);
    SetForegroundWindow(window);
  }
}
"@
$config = $env:SANSER_EXAM_GUARD | ConvertFrom-Json
$allowed = @($config.allowed | ForEach-Object { $_.ToLowerInvariant() })
$system = @($config.system | ForEach-Object { $_.ToLowerInvariant() })
$quiet = @($config.quiet | ForEach-Object { $_.ToLowerInvariant() })
$self = [int]$config.selfPid
$sites = @($config.aiTitles | ForEach-Object { [regex]::new($_, 'IgnoreCase') })

function Send($value) {
  [Console]::Out.WriteLine(($value | ConvertTo-Json -Compress))
  [Console]::Out.Flush()
}
function Matches-Any([string]$value, $words) {
  $lower = $value.ToLowerInvariant()
  foreach ($word in $words) { if ($lower.Contains($word)) { return $true } }
  return $false
}
function Bring-Back {
  foreach ($process in Get-Process) {
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) { continue }
    if ($allowed -contains ($process.ProcessName.ToLowerInvariant() + '.exe')) { [SanserGuard]::Bring($process.MainWindowHandle); return }
  }
  $sanser = Get-Process -Id $self -ErrorAction SilentlyContinue
  if ($sanser -and $sanser.MainWindowHandle -ne [IntPtr]::Zero) { [SanserGuard]::Bring($sanser.MainWindowHandle) }
}

Send @{ t = 'ready' }
$lastFront = $null
$lastTitle = $null
$tick = 0
while ($true) {
  $tick += 1
  if ($config.blockAi -and ($tick % 3) -eq 0) {
    foreach ($process in Get-Process) {
      if ($process.Id -eq $self -or $process.MainWindowHandle -eq [IntPtr]::Zero) { continue }
      $file = $process.ProcessName.ToLowerInvariant() + '.exe'
      if ($allowed -contains $file) { continue }
      $label = $process.ProcessName + ' ' + [string]$process.Description
      if (-not (Matches-Any $label $config.aiApps)) { continue }
      try { Stop-Process -Id $process.Id -Force } catch { }
      Send @{ t = 'ai-app'; d = $process.ProcessName }
    }
  }
  $window = [SanserGuard]::GetForegroundWindow()
  if ($window -ne [IntPtr]::Zero) {
    [uint32]$processId = 0
    [void][SanserGuard]::GetWindowThreadProcessId($window, [ref]$processId)
    $file = ''
    $name = ''
    try {
      $process = Get-Process -Id $processId
      $file = $process.ProcessName.ToLowerInvariant() + '.exe'
      $name = $process.ProcessName
    } catch { }
    $title = [SanserGuard]::Title($window)
    if ($file -ne $lastFront) { Send @{ t = 'front'; d = $(if ($title) { $name + ': ' + $title } else { $name }) }; $lastFront = $file; $lastTitle = $null }
    $isSelf = $processId -eq $self
    $ok = $isSelf -or $file -eq '' -or ($allowed -contains $file) -or ($system -contains $file)
    if (-not $ok) {
      # Clicking the taskbar or Start is not reported, only undone.
      if (-not (($quiet -contains $file) -and -not $title)) { Send @{ t = 'left-app'; d = $(if ($title) { $name + ': ' + $title } else { $name }) } }
      if (-not ($quiet -contains $file)) { [void][SanserGuard]::ShowWindow($window, 6) }
      Bring-Back
    } elseif ($config.blockAi -and -not $isSelf -and $title -and $title -ne $lastTitle) {
      $lastTitle = $title
      foreach ($site in $sites) {
        if ($site.IsMatch($title)) {
          Send @{ t = 'ai-site'; d = $name + ': ' + $title }
          $lastTitle = $null
          $sanser = Get-Process -Id $self -ErrorAction SilentlyContinue
          if ($sanser -and $sanser.MainWindowHandle -ne [IntPtr]::Zero) { [SanserGuard]::Bring($sanser.MainWindowHandle) }
          break
        }
      }
    }
  }
  Start-Sleep -Milliseconds ([int]$config.intervalMs)
}

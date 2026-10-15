param([string]$ExecutablePath = 'target/debug/sanser.exe')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$executable = (Resolve-Path $ExecutablePath).Path
$primary = $null
$secondary = $null
$vite = $null

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class SanserWindowSmoke {
    private delegate bool EnumWindow(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr window, StringBuilder title, int count);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr window, uint message, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    public static IntPtr FindMainWindow(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var title = new StringBuilder(256);
            GetWindowTextW(window, title, title.Capacity);
            if (!title.ToString().StartsWith("Sanser ", StringComparison.Ordinal) || !IsWindowVisible(window)) return true;
            found = window;
            return false;
        }, IntPtr.Zero);
        return found;
    }
}
'@

function Wait-Until([scriptblock]$Condition, [string]$Failure) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 100
    }
    throw $Failure
}

try {
    $start = [Diagnostics.ProcessStartInfo]::new('node')
    $start.WorkingDirectory = Join-Path $repo 'apps/desktop'
    $start.ArgumentList.Add((Join-Path $repo 'node_modules/vite/bin/vite.js'))
    foreach ($argument in @('--host', '127.0.0.1', '--port', '1420', '--strictPort')) { $start.ArgumentList.Add($argument) }
    $start.UseShellExecute = $false
    $vite = [Diagnostics.Process]::Start($start)
    Wait-Until {
        if ($vite.HasExited) { throw 'Vite exited before the Windows lifecycle test' }
        try { return (Invoke-WebRequest 'http://127.0.0.1:1420' -TimeoutSec 1).StatusCode -eq 200 } catch { return $false }
    } 'Vite did not start'

    $primary = Start-Process -FilePath $executable -PassThru
    Wait-Until {
        $primary.Refresh()
        if ($primary.HasExited) { throw 'Primary Sanser exited during startup' }
        return [SanserWindowSmoke]::FindMainWindow([uint32]$primary.Id) -ne [IntPtr]::Zero
    } 'Primary Sanser did not create its main window'
    $window = [SanserWindowSmoke]::FindMainWindow([uint32]$primary.Id)

    foreach ($attempt in 1..3) {
        if (-not [SanserWindowSmoke]::PostMessageW($window, 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)) { throw 'Unable to close Sanser to tray' }
        Wait-Until { -not [SanserWindowSmoke]::IsWindowVisible($window) } 'Closing Sanser did not hide its window'
        $secondary = Start-Process -FilePath $executable -PassThru
        if (-not $secondary.WaitForExit(30000) -or $secondary.ExitCode -ne 0) { throw 'Second launch did not exit successfully' }
        Wait-Until { [SanserWindowSmoke]::IsWindowVisible($window) } 'Second launch did not restore the original window'
        $primary.Refresh()
        if ($primary.HasExited) { throw 'Restoring the window replaced the original process' }
        $instances = @(Get-Process -Name sanser | Where-Object { $_.Path -eq $executable })
        if ($instances.Count -ne 1 -or $instances[0].Id -ne $primary.Id) { throw 'Duplicate Sanser processes remain' }
    }

    [void][SanserWindowSmoke]::ShowWindow($window, 6)
    Wait-Until { [SanserWindowSmoke]::IsIconic($window) } 'Unable to minimize the original window'
    $secondary = Start-Process -FilePath $executable -PassThru
    if (-not $secondary.WaitForExit(30000) -or $secondary.ExitCode -ne 0) { throw 'Launch while minimized did not exit successfully' }
    Wait-Until { -not [SanserWindowSmoke]::IsIconic($window) } 'Second launch did not unminimize the original window'
    Write-Output 'PASS: closing to tray and launching again preserves one process and restores the existing window; minimize/restore also passed.'
}
finally {
    foreach ($child in @($secondary, $primary, $vite)) {
        if ($null -ne $child -and -not $child.HasExited) { Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue }
    }
}

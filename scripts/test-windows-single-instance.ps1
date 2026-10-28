param([string]$ExecutablePath = 'target/debug/sanser.exe')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$executable = (Resolve-Path $ExecutablePath).Path
$primary = $null
$secondary = $null
$vite = $null
$logs = [Collections.Generic.List[string]]::new()

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class SanserWindowSmoke {
    private delegate bool EnumWindow(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr window, StringBuilder title, int count);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassNameW(IntPtr window, StringBuilder name, int count);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr window, uint message, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string className, string title);
    public static string ClassName(IntPtr window) { var name = new StringBuilder(256); GetClassNameW(window, name, name.Capacity); return name.ToString(); }
    public static IntPtr FindMainWindow(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var title = new StringBuilder(256);
            GetWindowTextW(window, title, title.Capacity);
            if (!title.ToString().StartsWith("Sanser ", StringComparison.Ordinal) || !IsWindowVisible(window)
                || ClassName(window) == "ConsoleWindowClass" || ClassName(window) == "CASCADIA_HOSTING_WINDOW_CLASS") return true;
            found = window;
            return false;
        }, IntPtr.Zero);
        return found;
    }
}
'@

function Wait-Until([scriptblock]$Condition, [string]$Failure, [int]$TimeoutSeconds = 30) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 100
    }
    throw $Failure
}

function Start-Sanser([string]$Name) {
    $path = Join-Path ([IO.Path]::GetTempPath()) "sanser-lifecycle-$Name.log"
    $logs.Add($path)
    return Start-Process -FilePath $executable -PassThru -RedirectStandardError $path
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

    $primary = Start-Sanser 'primary'
    # A cold Windows CI process can enter main near the normal 30s deadline.
    # Allow its initial setup more time without relaxing any restore/close or
    # duplicate-process assertion. Early process exit still fails immediately.
    Wait-Until {
        $primary.Refresh()
        if ($primary.HasExited) { throw 'Primary Sanser exited before setup completed' }
        return (Test-Path $logs[0]) -and [bool](Select-String -Path $logs[0] -SimpleMatch 'Sanser lifecycle: setup completed' -Quiet)
    } 'Primary Sanser did not complete setup' 60
    Wait-Until {
        $primary.Refresh()
        if ($primary.HasExited) { throw 'Primary Sanser exited during startup' }
        return [SanserWindowSmoke]::FindMainWindow([uint32]$primary.Id) -ne [IntPtr]::Zero
    } 'Primary Sanser did not create its main window'
    $window = [SanserWindowSmoke]::FindMainWindow([uint32]$primary.Id)
    Write-Output "Main window class: $([SanserWindowSmoke]::ClassName($window))"
    Write-Output "Instance guard window present: $([SanserWindowSmoke]::FindWindowW('com.sanser.desktop-sic', 'com.sanser.desktop-siw') -ne [IntPtr]::Zero)"

    foreach ($attempt in 1..3) {
        if (-not [SanserWindowSmoke]::PostMessageW($window, 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)) { throw 'Unable to close Sanser to tray' }
        Wait-Until { -not [SanserWindowSmoke]::IsWindowVisible($window) } 'Closing Sanser did not hide its window'
        Wait-Until { [bool](Select-String -Path $logs[0] -SimpleMatch 'Sanser lifecycle: hide requested' -Quiet) } 'Close request did not reach the Sanser main window handler'
        $primary.Refresh()
        if ($primary.HasExited) { throw 'Closing the main window terminated the original process' }
        Write-Output "Cycle ${attempt}: original window hidden; launching second process."
        $secondary = Start-Sanser "secondary-$attempt"
        if (-not $secondary.WaitForExit(30000)) { throw "Second launch timed out; originalVisible=$([SanserWindowSmoke]::IsWindowVisible($window)); secondaryPID=$($secondary.Id)" }
        if ($secondary.ExitCode -ne 0) { throw "Second launch returned exit code $($secondary.ExitCode)" }
        Wait-Until { [SanserWindowSmoke]::IsWindowVisible($window) } 'Second launch did not restore the original window'
        Wait-Until { @(Select-String -Path $logs[0] -SimpleMatch 'Sanser lifecycle: restore completed').Count -ge $attempt } 'Window restoration did not finish'
        $primary.Refresh()
        if ($primary.HasExited) { throw 'Restoring the window replaced the original process' }
        $instances = @(Get-Process -Name sanser | Where-Object { $_.Path -eq $executable })
        if ($instances.Count -ne 1 -or $instances[0].Id -ne $primary.Id) { throw 'Duplicate Sanser processes remain' }
        Write-Output "Cycle ${attempt}: original PID $($primary.Id) restored and second process exited."
    }

    [void][SanserWindowSmoke]::ShowWindow($window, 6)
    Wait-Until { [SanserWindowSmoke]::IsIconic($window) } 'Unable to minimize the original window'
    $secondary = Start-Sanser 'secondary-minimized'
    if (-not $secondary.WaitForExit(30000) -or $secondary.ExitCode -ne 0) { throw 'Launch while minimized did not exit successfully' }
    Wait-Until { -not [SanserWindowSmoke]::IsIconic($window) } 'Second launch did not unminimize the original window'
    Write-Output 'PASS: closing to tray and launching again preserves one process and restores the existing window; minimize/restore also passed.'
}
finally {
    foreach ($path in $logs) {
        if (Test-Path $path) { Get-Content $path | Where-Object { $_ -like '*Sanser lifecycle:*' } | Write-Output }
    }
    foreach ($child in @($secondary, $primary, $vite)) {
        if ($null -ne $child -and -not $child.HasExited) { Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue }
    }
}

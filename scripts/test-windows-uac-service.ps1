param([string]$Build = 'build/host-windows')
$ErrorActionPreference = 'Stop'
# Isolated Windows CI runner only. Never replace a user's installed service.
if (Get-Service -Name SanserUac -ErrorAction SilentlyContinue) { throw 'UAC test requires a clean runner' }
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ('sanser-uac-test-' + [Guid]::NewGuid())
New-Item -ItemType Directory $stage | Out-Null
$taskName = 'SanserUacUpdateTest-' + [Guid]::NewGuid()
$inputFailure = $null
try {
    Copy-Item "$Build/windows-uac/Release/sanser-uac-service.exe" "$stage/sanser-uac-service.exe"
    Copy-Item "$Build/Release/sanser-host-windows.exe" "$stage/sanser-host-windows.exe"
    & "$stage/sanser-uac-service.exe" --install
    if ($LASTEXITCODE -ne 0) { throw 'UAC service installation failed' }
    $status = (& "$stage/sanser-uac-service.exe" --status) | ConvertFrom-Json
    if (-not $status.enabled -or -not $status.current) { throw 'UAC service did not start with current protected binaries' }
    & "$Build/windows-uac/Release/sanser-uac-deny-test.exe"
    if ($LASTEXITCODE -ne 0) { throw 'UAC broker authorization failed' }
    $path = Join-Path $env:ProgramFiles 'SanserUac'
    $ownerBefore = Get-Content "$path/allowed-user.sid" -Raw
    & "$Build/windows-uac/Release/sanser-uac-input-test.exe" "$path/sanser-host-windows.exe"
    if ($LASTEXITCODE -eq 77) { Write-Warning 'Runner has no unlocked interactive desktop; actual elevated Cancel click was not exercised' }
    elseif ($LASTEXITCODE -ne 0) { $inputFailure = 'Elevated Cancel input regression failed' }

    # PE overlays do not change executable behavior, but reproduce a protected
    # helper/host left behind after app update, even when version strings match.
    $stream = [System.IO.File]::Open("$stage/sanser-host-windows.exe", [System.IO.FileMode]::Append)
    try { $stream.WriteByte(42) } finally { $stream.Dispose() }
    $status = (& "$stage/sanser-uac-service.exe" --status) | ConvertFrom-Json
    if (-not $status.enabled -or $status.current) { throw 'Stale protected host was reported ready' }
    $stream = [System.IO.File]::Open("$stage/sanser-uac-service.exe", [System.IO.FileMode]::Append)
    try { $stream.WriteByte(42) } finally { $stream.Dispose() }

    # Updaters may execute as SYSTEM. Exercise the actual installer command in
    # that context and prove it preserves the account authorized before update.
    $action = New-ScheduledTaskAction -Execute "$stage/sanser-uac-service.exe" -Argument '--installer-install'
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal | Out-Null
    Start-ScheduledTask -TaskName $taskName
    $deadline = (Get-Date).AddSeconds(35)
    do {
        Start-Sleep -Milliseconds 150
        $info = Get-ScheduledTaskInfo -TaskName $taskName
        $task = Get-ScheduledTask -TaskName $taskName
    } while (($info.LastRunTime.Year -lt 2020 -or $task.State -eq 'Running') -and (Get-Date) -lt $deadline)
    if ($info.LastTaskResult -ne 0 -or $task.State -eq 'Running') { throw 'Installer helper update as SYSTEM failed' }
    if ((Get-Content "$path/allowed-user.sid" -Raw) -ne $ownerBefore) { throw 'Installer updater rebound the UAC account to SYSTEM' }
    $status = (& "$stage/sanser-uac-service.exe" --status) | ConvertFrom-Json
    if (-not $status.enabled -or -not $status.current) { throw 'Installer update left stale protected binaries' }
    & "$Build/windows-uac/Release/sanser-uac-deny-test.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Updated UAC broker accepted unauthorized caller' }
    & "$stage/sanser-uac-service.exe" --uninstall
    if ($LASTEXITCODE -ne 0) { throw 'Explicit helper disable failed' }
    # A fresh task avoids treating two runs in the same clock second as one.
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    $taskName = 'SanserUacDisabledUpdateTest-' + [Guid]::NewGuid()
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal | Out-Null
    Start-ScheduledTask -TaskName $taskName
    $deadline = (Get-Date).AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 150
        $info = Get-ScheduledTaskInfo -TaskName $taskName
        $task = Get-ScheduledTask -TaskName $taskName
    } while (($info.LastRunTime.Year -lt 2020 -or $task.State -eq 'Running') -and (Get-Date) -lt $deadline)
    if ($info.LastTaskResult -ne 0 -or $info.LastRunTime.Year -lt 2020 -or (Get-Service -Name SanserUac -ErrorAction SilentlyContinue)) {
        throw 'SYSTEM updater ignored explicit helper disable'
    }
    & "$stage/sanser-uac-service.exe" --install
    if ($LASTEXITCODE -ne 0) { throw 'Explicit helper re-enable failed' }
    Stop-Service -Name SanserUac
    & "$stage/sanser-uac-service.exe" --installer-remove
    if ($LASTEXITCODE -ne 0 -or (Get-Service -Name SanserUac -ErrorAction SilentlyContinue)) {
        throw 'Installer uninstall left the stopped/failed registered helper behind'
    }
    & "$stage/sanser-uac-service.exe" --install
    if ($LASTEXITCODE -ne 0) { throw 'Helper lifecycle fixture restore failed' }
    $acl = Get-Acl $path
    if (-not $acl.AreAccessRulesProtected) { throw 'UAC binaries inherit an unsafe ACL' }
    foreach ($rule in $acl.Access) {
        if ($rule.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value -eq 'S-1-5-32-545' -and
            ($rule.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::WriteData)) {
            throw 'Users can overwrite the privileged helper'
        }
    }
    Write-Host 'UAC service lifecycle, stale-copy detection, SYSTEM update/account preservation, IPC denial and protected ACL passed'
    if ($inputFailure) { throw $inputFailure }
} finally {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    if (Test-Path "$stage/sanser-uac-service.exe") {
        & "$stage/sanser-uac-service.exe" --uninstall
        if ($LASTEXITCODE -ne 0) { throw 'UAC service cleanup failed' }
    }
    Remove-Item $stage -Recurse -Force
}
if (Get-Service -Name SanserUac -ErrorAction SilentlyContinue) { throw 'UAC test left a service installed' }

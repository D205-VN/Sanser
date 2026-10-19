param([string]$Build = 'build/host-windows')
$ErrorActionPreference = 'Stop'
# Isolated Windows CI runner only. Never replace a user's installed service.
if (Get-Service -Name SanserUac -ErrorAction SilentlyContinue) { throw 'UAC test requires a clean runner' }
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ('sanser-uac-test-' + [Guid]::NewGuid())
New-Item -ItemType Directory $stage | Out-Null
try {
    Copy-Item "$Build/windows-uac/Release/sanser-uac-service.exe" "$stage/sanser-uac-service.exe"
    Copy-Item "$Build/Release/sanser-host-windows.exe" "$stage/sanser-host-windows.exe"
    & "$stage/sanser-uac-service.exe" --install
    if ($LASTEXITCODE -ne 0) { throw 'UAC service installation failed' }
    $status = (& "$stage/sanser-uac-service.exe" --status) | ConvertFrom-Json
    if (-not $status.enabled) { throw 'UAC service did not start' }
    & "$Build/windows-uac/Release/sanser-uac-deny-test.exe"
    if ($LASTEXITCODE -ne 0) { throw 'UAC broker authorization failed' }
    $path = Join-Path $env:ProgramFiles 'SanserUac'
    $acl = Get-Acl $path
    if (-not $acl.AreAccessRulesProtected) { throw 'UAC binaries inherit an unsafe ACL' }
    foreach ($rule in $acl.Access) {
        if ($rule.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value -eq 'S-1-5-32-545' -and
            ($rule.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::WriteData)) {
            throw 'Users can overwrite the privileged helper'
        }
    }
    Write-Host 'UAC service installed/started; unauthorized IPC rejected; protected binary ACL verified'
} finally {
    if (Test-Path "$stage/sanser-uac-service.exe") {
        & "$stage/sanser-uac-service.exe" --uninstall
        if ($LASTEXITCODE -ne 0) { throw 'UAC service cleanup failed' }
    }
    Remove-Item $stage -Recurse -Force
}
if (Get-Service -Name SanserUac -ErrorAction SilentlyContinue) { throw 'UAC test left a service installed' }

# Tauri's single-attempt downloader can fail with HTTP 504 after a long build.
# Prepare the same official toolchain first, with bounded retries and pinned
# archive/plugin hashes. Tauri still performs its own validation before use.
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'NSIS preparation requires Windows' }
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ('sanser-nsis-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
function Get-VerifiedFile([string]$Url, [string]$Path, [string]$Sha256) {
    # GitHub's asset redirect can retain a failed edge response for this URL.
    # A fresh query obtains a fresh redirect; the pinned digest remains required.
    $freshUrl = $Url + '?sanser=' + [Guid]::NewGuid().ToString('N')
    & curl.exe --fail --location --silent --show-error --retry 4 --retry-all-errors --retry-delay 2 --connect-timeout 10 --max-time 60 --output $Path $freshUrl
    if ($LASTEXITCODE -ne 0) { throw 'Unable to download the official NSIS toolchain' }
    if ((Get-FileHash -Algorithm SHA256 $Path).Hash -ne $Sha256) { throw 'NSIS toolchain checksum mismatch' }
}
try {
    $archive = Join-Path $stage 'nsis.zip'
    Get-VerifiedFile 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip' $archive 'BA63DFFC4410EE89193E1CB5A41989991BD77C61068DA17E3156D136B7B0B3D8'
    Expand-Archive -Path $archive -DestinationPath $stage
    $extracted = Join-Path $stage 'nsis-3.13'
    if ((Get-FileHash -Algorithm SHA1 (Join-Path $extracted 'NSIS.exe')).Hash -ne '6D45D4E8CCF18FE1FDAC9164058F356102A2ECF2') { throw 'NSIS executable differs from the Tauri toolchain' }
    $plugins = Join-Path $extracted 'Plugins/x86-unicode/additional'
    New-Item -ItemType Directory -Path $plugins -Force | Out-Null
    Get-VerifiedFile 'https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll' (Join-Path $plugins 'nsis_tauri_utils.dll') '5BA143B5DB4A87D32D6E7802E033330AAE56CBCEABE0D1E3BA41948385AD4709'
    $cache = Join-Path $env:LOCALAPPDATA 'tauri'
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    $destination = Join-Path $cache 'NSIS'
    if (Test-Path $destination) { Remove-Item $destination -Recurse -Force }
    Move-Item $extracted $destination
    Write-Host 'Official NSIS 3.13 and Tauri plugin verified and staged'
} finally {
    Remove-Item $stage -Recurse -Force
}

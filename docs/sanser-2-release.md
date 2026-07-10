# Sanser 2.0.0 release record

## Identity

- Product: Sanser
- Version: 2.0.0
- Application ID: `com.sanser.desktop`
- Protocol: v2 / SNV2

## Required artifacts

- `Sanser-Windows-2.0.0-x64-portable.exe`
- `Sanser-Windows-2.0.0-x64-setup.exe`
- `Sanser-macOS-2.0.0-arm64.dmg`
- `Sanser-macOS-2.0.0-arm64.zip`
- SHA-256 checksums and release notes

## Verification ledger

| Item | Status |
| --- | --- |
| Portable C++ SNV2 header codec/test on macOS | Verified locally |
| Tauri/Svelte desktop | In implementation; release build not yet recorded |
| Rust API/local storage | In implementation; full integration not yet recorded |
| Windows host build | Not verified in this macOS workspace |
| macOS legacy media engine build after directory migration | Verified; full SNV2 integration pending |
| TURN relay | Not verified without a TURN test deployment |
| Signing/notarization | Not configured with a real certificate |

The release is not marked complete until CI and the platform/hardware matrix pass. No benchmark numbers are claimed yet; use `docs/sanser-2-performance.md` for the measurement contract.

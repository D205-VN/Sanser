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
- Signed platform updater bundles and `updater-latest.json`
- SHA-256 checksums and release notes

## Verification ledger

| Item | Status |
| --- | --- |
| Portable C++ SNV2 header codec/test on macOS | Verified locally |
| Svelte production frontend | Type-check, lint, 10 tests and production build verified locally on 2026-07-11 |
| Tauri macOS shell | Cargo check and strict Clippy verified locally; signed/notarized bundle not recorded |
| Rust API / Neon PostgreSQL | Unit/API tests passed; live Neon startup, migrations, health and readiness verified on 2026-07-11 |
| Windows host build | Not verified in this macOS workspace |
| macOS native media engine | Build and capability probe verified; H.264/HEVC/Metal/audio/input implementations present, full shared SNV2 integration pending |
| Static updater manifest workflow | Implemented; requires a successful signed multi-platform release before end-to-end verification |
| TURN relay | Not verified without a TURN test deployment |
| Signing/notarization | Not configured with a real certificate |

The release is not marked complete until CI and the platform/hardware matrix pass. No benchmark numbers are claimed yet; use `docs/sanser-2-performance.md` for the measurement contract.

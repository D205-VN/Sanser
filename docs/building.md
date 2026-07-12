# Building Sanser 2.0.2

## macOS arm64

```bash
npm run native:client-macos:configure
npm run native:client-macos:build
npm run desktop:bundle
```

The release target produces an app bundle, DMG, and updater-independent archive when Tauri bundle prerequisites are present. Hardened runtime/signing configuration is prepared in `apps/desktop/src-tauri`; an unsigned local build is not notarized.

## Windows x64

From a Visual Studio Developer PowerShell:

```powershell
npm run native:host-windows:configure
npm run native:host-windows:build
npm run desktop:bundle
```

The Windows bundle must contain only the Tauri shell, `sanser-host-windows`, WebView2 bootstrap policy, and production assets. It must not contain the API server, database drivers, macOS engine, source trees, tests, CMake cache, captures, Node modules, or debug symbols.

See `docs/sanser-2-release.md` for artifact names and verification status.

## Native sidecar diagnostics

Sidecar stdout and stderr are discarded by default. For a local debugging run,
set `SANSER_SIDECAR_DEBUG_LOG=1`; the desktop then truncates and writes one
private file in the operating-system application log directory. Native logs may
contain peer endpoint details, so do not attach them publicly without review.

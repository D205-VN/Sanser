# Native modules by operating system

The media engines use separate platform libraries. The account/settings workspace
remains a Tauri/Svelte application; this change does not replace it with SwiftUI
or WinUI. Remote video windows and OS input are native, outside the WebView.

| Module | Responsibility | Build target |
| --- | --- | --- |
| `native/platforms/macos` | AppKit window/input, ScreenCaptureKit capture, VideoToolbox codec, Metal presentation | `sanser-native-macos` |
| `native/platforms/windows` | Win32 window/input, Desktop Duplication capture, Media Foundation codec | `sanser-native-windows` |
| `native/desktop` | Shared authenticated transport, engine entry points and transport tests | `sanser-desktop-transport` |
| `native/protocol` | Packet format, cryptography and STUN | `sanser-snv2`, `sanser-stun` |

Each package still includes two executables: `sanser-client-macos` and
`sanser-host-macos` on macOS; `sanser-client-windows.exe` and
`sanser-host-windows.exe` on Windows. Executable names and staging locations are
unchanged. CMake selects only the current OS backend. The existing legacy
macOS-client/Windows-host implementation remains in its original directories.

## Rendering changes

For SNV2 sessions, the Mac client now presents decoded `CVPixelBuffer` frames
through an `MTKView` and a reusable Metal-backed `CIContext`. It no longer creates
an intermediate `CGImage` and `NSImage` for each frame. Presentation preserves
aspect ratio and image orientation, keeps only the newest decoded image, and
limits outstanding GPU submissions to two. The GPU retains each submitted pixel
buffer until completion. Redrawing happens on new frames or view invalidation.
This follows Apple's [Core Image texture rendering API](https://developer.apple.com/documentation/coreimage/cicontext/render(_:to:commandbuffer:bounds:colorspace:))
and [MTKView](https://developer.apple.com/documentation/metalkit/mtkview/) interfaces.

The Windows SNV2 client waits for frame notifications or window messages rather
than sleeping and invalidating the entire window every 16 ms. An auto-reset event
coalesces frame notifications; painting holds an immutable frame snapshot and
releases the shared mutex before drawing, so painting does not lock out decoded
frames. NV12 conversion and GDI presentation are still CPU-based; this change does
not claim a full Direct3D decoder/presentation pipeline on Windows.

These are code-path improvements, not measured end-to-end FPS or latency gains.
The shared transport and its packet-loss limitations have not changed.

## Local validation

- macOS ARM64 host/client build and capability probes.
- macOS x86_64 host/client cross-compilation (not execution on an Intel Mac).
- Six native tests: wheel input, protocol/authentication, STUN, transport,
  encrypted UDP loopback, and VideoToolbox codec plus offscreen Metal presentation.
- The Metal test checks two distinct horizontal color bands, correct vertical
  orientation, and black aspect-ratio bars using an actual GPU texture readback.
- Production sidecars are built with `BUILD_TESTING=OFF`.

Windows compilation/execution, actual remote desktop window behavior, screen
capture permissions, and physical two-computer network performance still require
verification on the respective devices. Existing CI builds both OS backends and
runs their native tests. No production server or installer is deployed here.

```sh
# Build and stage the current OS engines for the desktop application.
npm run desktop:stage-sidecar

# macOS native tests (GPU and local UDP sockets required).
cmake -S native/client-macos -B /tmp/sanser-platform-native-build \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=ON
cmake --build /tmp/sanser-platform-native-build --parallel
ctest --test-dir /tmp/sanser-platform-native-build --output-on-failure

# Windows native tests, in a Visual Studio developer terminal.
cmake -S native/host-windows -B build/host-windows -A x64 -DBUILD_TESTING=ON
cmake --build build/host-windows --config Release --parallel
ctest --test-dir build/host-windows -C Release --output-on-failure
```

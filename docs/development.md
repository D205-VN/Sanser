# Development

Requirements:

- Rust stable (MSRV 1.85 or newer)
- Node.js 20.19 or newer and npm 11
- CMake 3.20 or newer
- macOS: Xcode command-line tools
- Windows: Visual Studio 2022 C++ desktop workload and Windows SDK

Install frontend development dependencies with `npm install`. Node is used by Vite/Svelte tooling only; it is not packaged as a Sanser runtime.

Common checks:

```bash
npm run check
npm run test
npm run build
npm run native:protocol:test
```

These npm commands place Cargo output in the platform cache rather than in the
repository. This avoids partial `.rlib`/`.rmeta` artifacts when the workspace
is stored in a cloud-synced folder. Set `CARGO_TARGET_DIR` explicitly to
override the cache location.

Set a TLS-enabled Neon `DATABASE_URL`, then run the Rust API server with `npm run server:dev`. Run the WebView frontend with `npm run dev`, or the complete Tauri shell with `npm run desktop:dev`. The desktop does not bundle a database or local API sidecar.

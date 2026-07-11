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
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
npm run desktop:check
npm run desktop:lint
npm run desktop:test
npm run desktop:build
npm run native:protocol:test
```

Set a TLS-enabled Neon `DATABASE_URL`, then run the Rust API server with `npm run server:dev`. Run the WebView frontend with `npm run dev`, or the complete Tauri shell with `npm run desktop:dev`. The desktop does not bundle a database or local API sidecar.

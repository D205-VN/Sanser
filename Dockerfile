# ── Build stage ──────────────────────────────────────────────
FROM rust:1.86-bookworm AS builder

WORKDIR /app

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY apps/server/Cargo.toml apps/server/Cargo.toml
COPY apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/Cargo.toml
COPY crates/ crates/

# Create stub source files so cargo can fetch + compile dependencies
RUN mkdir -p apps/server/src && \
    echo 'fn main() {}' > apps/server/src/main.rs && \
    touch apps/server/src/lib.rs && \
    mkdir -p apps/desktop/src-tauri/src && \
    echo 'fn main() {}' > apps/desktop/src-tauri/src/main.rs && \
    touch apps/desktop/src-tauri/src/lib.rs

# Build dependencies only (cached layer)
RUN cargo build --release -p sanser-server 2>/dev/null || true

# Copy actual server source and rebuild
COPY apps/server/ apps/server/

# Touch source files so cargo detects the change
RUN touch apps/server/src/main.rs apps/server/src/lib.rs

RUN cargo build --release -p sanser-server

# ── Runtime stage ────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sanser-server /usr/local/bin/sanser-server

WORKDIR /app

ENV RUST_LOG=sanser_server=info,tower_http=info

EXPOSE 10000

CMD ["sanser-server"]

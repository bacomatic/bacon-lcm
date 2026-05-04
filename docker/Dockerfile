# docker/Dockerfile
# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependency compilation separately from source changes.
# Copy manifests first so the dep layer is only rebuilt when deps change.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml       core/Cargo.toml
COPY daemon/Cargo.toml     daemon/Cargo.toml
COPY mcp-server/Cargo.toml mcp-server/Cargo.toml
COPY cli/Cargo.toml        cli/Cargo.toml

# Create stub lib/main files so `cargo build` can resolve the workspace.
RUN mkdir -p core/src daemon/src mcp-server/src cli/src \
    && echo 'fn main(){}' > daemon/src/main.rs \
    && echo 'fn main(){}' > mcp-server/src/main.rs \
    && echo 'fn main(){}' > cli/src/main.rs \
    && echo ''            > core/src/lib.rs \
    && echo ''            > daemon/src/lib.rs \
    && echo ''            > mcp-server/src/lib.rs

RUN cargo build --release 2>/dev/null; true

# Now copy real source and build for real.
COPY . .
RUN touch core/src/lib.rs daemon/src/lib.rs mcp-server/src/lib.rs \
         daemon/src/main.rs mcp-server/src/main.rs cli/src/main.rs
RUN cargo build --release

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/bacon-lcm-daemon     /usr/local/bin/bacon-lcm-daemon
COPY --from=builder /app/target/release/bacon-lcm-mcp-server /usr/local/bin/bacon-lcm-mcp-server

EXPOSE 3333

CMD ["bacon-lcm-daemon"]

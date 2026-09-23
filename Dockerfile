# ==========================================
# Stage 1: Build Stage
# ==========================================
FROM rust:latest as builder

WORKDIR /usr/src/htar-gateway

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml ./
COPY crates ./crates

# Build release binary for htar-cli
RUN cargo build --release -p htar-cli

# ==========================================
# Stage 2: Production Minimal Runtime (Matching GLIBC 2.39)
# ==========================================
FROM debian:trixie-slim

WORKDIR /app

# Install runtime SSL CA certificates and MySQL client
RUN apt-get update && apt-get install -y ca-certificates curl mariadb-client && rm -rf /var/lib/apt/lists/*

# Create non-root system user
RUN useradd -m -u 10001 -s /bin/sh htargw

# Copy release binary and sample config
COPY --from=builder /usr/src/htar-gateway/target/release/htar-gw /usr/local/bin/htar-gw
COPY config/gateway.toml /app/config/gateway.toml

# Set permissions
RUN chown -R htargw:htargw /app
USER htargw

# Expose Gateway Traffic Port & Admin API Port
EXPOSE 8443

# Health check probe against gateway health endpoint
HEALTHCHECK --interval=10s --timeout=3s --retries=3 \
  CMD curl -f http://localhost:8443/_htar/health || exit 1

ENTRYPOINT ["/usr/local/bin/htar-gw"]
CMD ["run", "--config", "/app/config/gateway.toml"]

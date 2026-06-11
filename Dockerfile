# syntax=docker/dockerfile:1
#
# Production image for the TimeTracker API server (Axum + Postgres + SQLx).
#
# Builds OFFLINE: the SQLx `query!` macros are checked against the committed
# `server/.sqlx/` cache, so no database is needed at build time. Regenerate the
# cache whenever queries change with:
#
#     cd server && DATABASE_URL=... cargo sqlx prepare
#
# Migrations are embedded into the binary at compile time (sqlx::migrate!), so
# the runtime image ships no source files — just the binary + TLS roots.

############################
# 1. Builder
############################
FROM rust:1-bookworm AS builder
WORKDIR /app

# Compile-time query verification uses the checked-in .sqlx cache, not a live DB.
ENV SQLX_OFFLINE=true

# Copy the workspace source (see .dockerignore for what's excluded).
COPY . .

# This image ships ONLY the API server. Drop the Tauri desktop crate from the
# workspace so its GUI dependencies are never resolved, fetched, or built.
RUN sed -i '\#apps/desktop/src-tauri#d' Cargo.toml

# Build the server. BuildKit cache mounts keep the cargo registry and target dir
# warm across builds; the binary is copied out of the cached target dir in-layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p server \
    && cp target/release/server /usr/local/bin/server

############################
# 2. Runtime
############################
FROM debian:bookworm-slim AS runtime

# TLS roots for outbound HTTPS (Linear, Gemini, Cloudflare R2).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user.
RUN useradd --system --uid 10001 appuser

COPY --from=builder /usr/local/bin/server /usr/local/bin/server

USER appuser
ENV SERVER_HOST=0.0.0.0
EXPOSE 8090

# Bind the port the platform injects (Render/Heroku set $PORT), falling back to
# SERVER_PORT, then 8090. All other config comes from the environment at runtime
# (DATABASE_URL, JWT secrets, S3/R2, LINEAR_API_KEY, GEMINI_API_KEY, …).
CMD ["/bin/sh", "-c", "exec env SERVER_PORT=\"${PORT:-${SERVER_PORT:-8090}}\" /usr/local/bin/server"]

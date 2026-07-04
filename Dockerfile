# syntax=docker/dockerfile:1

# ---- Web build --------------------------------------------------------------
# Build the embedded SPA (web/dist/index.html) in a Node stage so the Rust
# builder doesn't need a JS toolchain and dependency layers cache well.
FROM node:22-slim AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml* web/.npmrc* ./
RUN pnpm install --frozen-lockfile || pnpm install
COPY web/ ./
# pnpm 11 blocks a plain `pnpm build`; invoke Vite directly.
RUN node node_modules/vite/bin/vite.js build

# ---- Builder ----------------------------------------------------------------
# rust:1-slim + build-essential gives us the C compiler that the bundled SQLite
# and the LAME MP3 encoder (mp3lame-encoder builds LAME via `cc`) need.
FROM rust:1-slim AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the manifests first so dependency compilation is cached separately from
# the source, then copy the source. The prebuilt SPA comes from the web stage;
# TIDAL_SUBSONIC_SKIP_WEB_BUILD tells build.rs to use it instead of re-building.
COPY Cargo.toml Cargo.lock* build.rs ./
COPY src ./src
COPY --from=web /web/dist ./web/dist
COPY web/package.json ./web/package.json

ENV TIDAL_SUBSONIC_SKIP_WEB_BUILD=1
RUN cargo build --release

# ---- Runtime ----------------------------------------------------------------
# debian:stable-slim ships a glibc compatible with the dynamically-linked
# binary produced above.
FROM debian:stable-slim AS runtime

# TLS for outbound HTTPS to TIDAL, plus CA certificates.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/tidal-subsonic /usr/local/bin/tidal-subsonic

# The `dirs` crate honors XDG_CONFIG_HOME / XDG_CACHE_HOME on Linux, so we point
# both at fixed, mountable locations:
#   database    -> /config/tidal-subsonic/tidal-subsonic.db
#   media cache -> /cache/tidal-subsonic/media/
ENV XDG_CONFIG_HOME=/config \
    XDG_CACHE_HOME=/cache

# Set TIDAL_SUBSONIC_KEY (base64 32-byte key) at runtime to keep the token
# encryption key stable across container rebuilds. If unset, a key is generated
# and stored in the /config volume on first run.
#   -e TIDAL_SUBSONIC_KEY="$(head -c 32 /dev/urandom | base64)"
#   -e TIDAL_SUBSONIC_ADMIN_PASSWORD="..."   (else printed to the log once)

RUN mkdir -p /config /cache
VOLUME ["/config", "/cache"]

EXPOSE 4533

ENTRYPOINT ["/usr/local/bin/tidal-subsonic"]

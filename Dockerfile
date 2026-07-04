# syntax=docker/dockerfile:1

# ---- Web build --------------------------------------------------------------
# Build the embedded SPA (web/dist/index.html) in a Node stage so the Rust
# builder doesn't need a JS toolchain and dependency layers cache well.
FROM node:22-slim AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml* web/.npmrc* ./
# pnpm 11 exits non-zero when a dep build script is "ignored" (esbuild); our
# build doesn't need esbuild's postinstall, so downgrade that to a warning.
RUN pnpm install --frozen-lockfile --config.strict-dep-builds=false
COPY web/ ./
# pnpm 11 blocks a plain `pnpm build`; invoke Vite directly.
RUN node node_modules/vite/bin/vite.js build

# ---- Builder ----------------------------------------------------------------
# Static musl build so the binary can run on distroless/static (no libc). Alpine
# ships a native musl C toolchain, which the bundled SQLite and the LAME MP3
# encoder (mp3lame-encoder builds LAME via `cc`) compile against cleanly.
# reqwest uses rustls with bundled webpki roots, so there's no OpenSSL to link
# and no system CA store to ship.
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev gcc make

WORKDIR /app

# Copy the manifests first so dependency compilation caches separately from the
# source. The prebuilt SPA comes from the web stage; TIDAL_SUBSONIC_SKIP_WEB_BUILD
# tells build.rs to use it rather than re-running the frontend build.
COPY Cargo.toml Cargo.lock* build.rs ./
COPY src ./src
COPY --from=web /web/dist ./web/dist
COPY web/package.json ./web/package.json

ENV TIDAL_SUBSONIC_SKIP_WEB_BUILD=1
RUN cargo build --release

# ---- Runtime ----------------------------------------------------------------
# distroless/static: no shell, no package manager, no libc — just the static
# binary. Tiny (~16MB total) and minimal attack surface. CA certificates aren't
# needed: rustls verifies TIDAL's TLS against webpki roots baked into the binary.
FROM gcr.io/distroless/static-debian12 AS runtime

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

VOLUME ["/config", "/cache"]

EXPOSE 4533

ENTRYPOINT ["/usr/local/bin/tidal-subsonic"]

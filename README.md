<div align="center">

# 🌊 tidal-subsonic

**A [Subsonic](https://www.subsonic.org/pages/api.jsp)-compatible API server backed by [TIDAL](https://tidal.com).**

Point any Subsonic client at it and stream your TIDAL library — HI-RES lossless included — from apps you already use.

[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![axum](https://img.shields.io/badge/axum-0.7-6E4AFF)](https://github.com/tokio-rs/axum)
[![Subsonic API](https://img.shields.io/badge/Subsonic%20API-1.16.1-1DB954)](https://www.subsonic.org/pages/api.jsp)
[![OpenSubsonic](https://img.shields.io/badge/OpenSubsonic-aware-2f6f9e)](https://opensubsonic.netlify.app/)
[![Multi-user](https://img.shields.io/badge/multi--user-yes-0a7d6b)](#multi-user)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](#license)

</div>

---

tidal-subsonic speaks the Subsonic REST API (JSON **and** XML), translates to TIDAL's APIs on the backend, and is **multi-user** from the ground up — each person links their own TIDAL account, and every credential is **encrypted at rest**. It handles the hard parts of proxying TIDAL: reassembling HI-RES DASH streams into seekable files, transcoding to MP3 for watch apps that require it, and caching aggressively so repeat plays are instant.

Written in Rust with [axum](https://github.com/tokio-rs/axum).

## Contents

- [Features](#features)
- [How it works](#how-it-works)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [First run &amp; admin](#first-run--admin)
- [The web portal](#the-web-portal)
- [Managing users](#managing-users)
- [Connecting a client](#connecting-a-client)
- [Supported endpoints](#supported-endpoints)
- [Docker](#docker)
- [Security](#security)
- [Building from source](#building-from-source)
- [License](#license)

## Features

| | |
|---|---|
| 🎧 **Full Subsonic REST API** | JSON &amp; XML (`f=json` / `f=xml`), OpenSubsonic-aware, served at both `/rest/<name>` and `/rest/<name>.view`. |
| 💿 **HI-RES streaming** | TIDAL HI-RES/lossless is reassembled from DASH segments into a single seekable file, with full HTTP range support so scrubbing works. |
| 🔁 **On-demand MP3 transcode** | Serves AAC by default; `format=mp3` transparently transcodes (pure-Rust: symphonia decode → LAME encode) for clients that only speak MP3, like the Garmin YuMusic watch app. |
| ⚡ **Two-tier caching** | A disk cache of assembled/transcoded audio (a replay or seek never re-fetches from TIDAL) plus a per-user in-memory cache of browse/search responses. |
| 🎵 **Rich library** | Artists, albums, tracks, genres, search, cover art, top &amp; similar songs, artist/album info with biographies. |
| 📝 **Editable playlists** | List, view, **create, add/remove tracks, rename, and delete** — mapped to TIDAL's playlist APIs. |
| ⭐ **Favorites &amp; lyrics** | `star`/`unstar` map to TIDAL favorites; `getLyrics` / `getLyricsBySongId` return synced, timestamped lyrics. |
| ⏯️ **Resume anywhere** | Server-side play queue and per-track bookmarks (`savePlayQueue`, `createBookmark`) sync across devices. |
| 👥 **Multi-user** | Admin-created accounts; each user links their own TIDAL account via a browser OAuth flow; per-user data isolation. |
| 🔐 **Security-hardened** | Encrypted tokens &amp; passwords at rest, POST-based account linking, SSRF/XSS/path-traversal protection, log redaction, secure first-run admin bootstrap. |

## How it works

```
┌─────────────┐   Subsonic API    ┌───────────────────┐   TIDAL APIs    ┌─────────┐
│  Subsonic   │  (JSON / XML)     │  tidal-subsonic   │  (private v1/v2 │  TIDAL  │
│  client     │◀─────────────────▶│                   │◀───────────────▶│  cloud  │
│ (phone,     │   /rest/*         │  • per-user auth  │   + OAuth       │         │
│  watch, PC) │                   │  • DASH reassembly│                 │         │
└─────────────┘                   │  • MP3 transcode  │                 └─────────┘
                                  │  • disk + memory  │
                                  │    caches         │      ┌──────────────┐
                                  │  • SQLite: users, │◀────▶│  SQLite DB   │
                                  │    tokens (enc.)  │      │  + disk cache│
                                  └───────────────────┘      └──────────────┘
```

A few things worth knowing:

- **TIDAL delivers audio as segmented DASH**, not a single file. tidal-subsonic downloads and concatenates the segments into one playable, seekable stream on the fly (and caches the result).
- **Garmin and similar watch apps only accept MP3.** When a client sends `format=mp3`, the server decodes the AAC and re-encodes to MP3 in-process — no external `ffmpeg` needed.
- **Content is shared; libraries are personal.** A track's audio is identical for everyone, so the media cache is shared across users; favorites and playlists are per-account.

## Quick start

> [!IMPORTANT]
> Always run the **release** build. MP3 transcoding is roughly **4× slower** in a debug build.

```sh
cargo build --release
./target/release/tidal-subsonic
```

The server listens on **port 4533**. On first run it creates its database and prints a generated admin password to the log — see [First run &amp; admin](#first-run--admin).

> [!NOTE]
> **Build requirements:** a **C compiler** (`gcc`/`clang`, or `build-essential` on Debian/Ubuntu) — the MP3 encoder (`mp3lame-encoder`) compiles LAME from source and the bundled SQLite needs it too — plus **Node + pnpm**, since `build.rs` builds the embedded web portal (`web/`) as part of `cargo build`. To build the Rust binary against a prebuilt portal (no Node), set `TIDAL_SUBSONIC_SKIP_WEB_BUILD=1` and provide `web/dist/index.html`.

Prefer not to build locally? Jump to [Docker](#docker).

## Configuration

### Environment variables

| Variable | Purpose |
|---|---|
| `TIDAL_SUBSONIC_KEY` | Base64-encoded **32-byte** key that encrypts TIDAL tokens and Subsonic passwords at rest. **Set this yourself** so it's stable across restarts and container rebuilds. If unset, a key is generated on first run and stored in the database. |
| `TIDAL_SUBSONIC_ADMIN_PASSWORD` | Password for the `admin` user created on first run. If unset, a random password is generated and printed to the log **once**. |
| `RUST_LOG` | Standard [`tracing`](https://docs.rs/tracing-subscriber) filter, e.g. `RUST_LOG=info` or `RUST_LOG=tidal_subsonic=debug`. |

Generate a stable encryption key:

```sh
head -c 32 /dev/urandom | base64
```

### Data locations

The database (users, encrypted tokens, config) lives in your OS **config** directory; the on-disk media cache lives in your OS **cache** directory. Both are created automatically.

| OS | Database | Media cache |
|---|---|---|
| **Linux** | `~/.config/tidal-subsonic/tidal-subsonic.db` | `~/.cache/tidal-subsonic/media/` |
| **macOS** | `~/Library/Application Support/tidal-subsonic/…` | `~/Library/Caches/tidal-subsonic/media/` |

When running in Docker, mount these as volumes to persist them — see [Docker](#docker).

## First run &amp; admin

On first launch the server bootstraps an **`admin`** user:

- If `TIDAL_SUBSONIC_ADMIN_PASSWORD` is set, that becomes the admin password.
- Otherwise a random password is generated and **printed to the log once**:

  ```
  First run: created admin user 'admin' with generated password: <password>
  ```

  Copy it immediately. You can change it later with `changePassword`, or set `TIDAL_SUBSONIC_ADMIN_PASSWORD` before the first run to choose your own.

## The web portal

tidal-subsonic ships a **built-in web portal** (a single-page app embedded in the binary — no separate service). Open **`http://<host>:4533/`** and sign in with your Subsonic credentials to:

- **Link your TIDAL account** — walks you through TIDAL's OAuth: open the login page, paste the code, done. Re-link or unlink any time.
- **See your connection details** — server URL + username with copy buttons, and how to add them to a Subsonic client.
- **Change your password.**
- **Manage users** (admins only) — a table of accounts with create, delete, promote/demote, and reset-password.

The portal is backed by a small JSON `/api/*` surface with cookie sessions (HttpOnly, SameSite=Strict); it's separate from the Subsonic `/rest/*` API. Until a user links TIDAL, their Subsonic browse calls return *"Not authenticated with TIDAL."*

## Managing users

Any admin can manage accounts through standard Subsonic endpoints. Every call takes the usual auth params (`u`, `t`+`s` or `p`, `c`, `v`, `f`).

```sh
# Create a user
curl "http://localhost:4533/rest/createUser?u=admin&p=<adminpass>&c=cli&v=1.16.1&f=json&username=alice&password=hunter2"

# List users  ·  update  ·  delete  ·  change a password
curl "http://localhost:4533/rest/getUsers?u=admin&p=<adminpass>&c=cli&v=1.16.1&f=json"
```

Then have that user open `http://<host>:4533/` to link their own TIDAL account.

## Connecting a client

Configure your Subsonic client with:

- **Server / URL:** `http://<host>:4533`
- **Username / Password:** the Subsonic account credentials (admin, or one you created)

### Client notes

- **Garmin YuMusic** (and other MP3-only clients) — these declare only MP3 support and it's handled automatically: when the client requests `format=mp3`, audio is transcoded to MP3 on the fly. No configuration needed.
- **Submariner** — works, including playlist editing. Note it caches tracks locally and won't re-apply metadata to a track it already knows; if titles/durations look blank after linking, remove and re-add the server so it rebuilds its cache.
- **Streaming quality** — HI-RES/lossless is served via DASH reassembly by default; clients that negotiate Subsonic transcoding get MP3 when they ask for it.

## Supported endpoints

<details open>
<summary><b>System &amp; discovery</b></summary>

| Endpoint | |
|---|---|
| `ping`, `getLicense` | Liveness &amp; license |
| `getOpenSubsonicExtensions` | OpenSubsonic capability advertisement |
| `getScanStatus`, `startScan` | Library scan (no-op; the proxy is always "scanned") |
| `scrobble`, `getNowPlaying` | Playback reporting |

</details>

<details open>
<summary><b>Browsing &amp; search</b></summary>

| Endpoint | |
|---|---|
| `getMusicFolders`, `getIndexes`, `getGenres` | Roots |
| `getArtists`, `getArtist`, `getAlbum`, `getSong` | ID3 browsing |
| `getMusicDirectory` | Folder-style browsing (classic clients) |
| `getAlbumList`, `getAlbumList2`, `getRandomSongs` | Lists |
| `search2`, `search3` | Search |
| `getArtistInfo`, `getArtistInfo2`, `getAlbumInfo`, `getAlbumInfo2` | Biographies &amp; images |
| `getTopSongs`, `getSimilarSongs`, `getSimilarSongs2` | Discovery |

</details>

<details open>
<summary><b>Media &amp; artwork</b></summary>

| Endpoint | |
|---|---|
| `stream`, `download` | Audio — supports `format=mp3`, range/seek |
| `getCoverArt`, `getAvatar` | Artwork |
| `getLyrics`, `getLyricsBySongId` | Synced lyrics |

</details>

<details open>
<summary><b>Library actions</b></summary>

| Endpoint | |
|---|---|
| `getStarred`, `getStarred2`, `star`, `unstar` | Favorites |
| `getPlaylists`, `getPlaylist` | Playlist read |
| `createPlaylist`, `updatePlaylist`, `deletePlaylist` | Playlist write |
| `savePlayQueue`, `getPlayQueue` | Cross-device play queue |
| `createBookmark`, `getBookmarks`, `deleteBookmark` | Per-track bookmarks |

</details>

<details open>
<summary><b>Users</b></summary>

| Endpoint | |
|---|---|
| `getUser`, `getUsers` | Read |
| `createUser`, `updateUser`, `deleteUser`, `changePassword` | Management (admin-gated) |

</details>

> Anything outside a TIDAL proxy's scope (podcasts, internet radio, video, jukebox, shares) is intentionally unimplemented and returns a clean Subsonic error.

## Docker

A multi-stage `Dockerfile` is included (the builder installs the C compiler; the runtime is `debian:stable-slim`).

```sh
docker build -t tidal-subsonic .

docker run -d --name tidal-subsonic \
  -p 4533:4533 \
  -e TIDAL_SUBSONIC_KEY="$(head -c 32 /dev/urandom | base64)" \
  -e TIDAL_SUBSONIC_ADMIN_PASSWORD="choose-a-strong-password" \
  -v tidal-subsonic-config:/config \
  -v tidal-subsonic-cache:/cache \
  tidal-subsonic
```

- The image sets `XDG_CONFIG_HOME=/config` and `XDG_CACHE_HOME=/cache`, so the database lands in `/config/tidal-subsonic/` and the media cache in `/cache/tidal-subsonic/`. **Mount both** to persist data.
- **Set `TIDAL_SUBSONIC_KEY` explicitly** so the encryption key survives rebuilds. Without it, a new key is generated in the `/config` volume on first run.
- If you didn't set `TIDAL_SUBSONIC_ADMIN_PASSWORD`, grab the generated one from `docker logs tidal-subsonic`.

## Security

- **Encryption at rest** — TIDAL tokens and Subsonic passwords are encrypted with ChaCha20-Poly1305 using `TIDAL_SUBSONIC_KEY` (or a DB-stored generated key). Keep the key stable; losing it makes every stored credential unreadable.
- **POST-based linking** — the account-linking page sends Subsonic passwords in the POST body, so they never land in URLs, query strings, or logs.
- **SSRF protection** — outbound fetches (cover art, stream segments) are host-checked and reject loopback, private, link-local, and unspecified addresses.
- **Path-traversal safe** — media cache filenames are content-hashed, so no request-supplied value can escape the cache directory.
- **Log redaction** — credentials and tokens are stripped from request logs.
- **Passwords are recoverable by design** — Subsonic token auth is `md5(password + salt)` with a client-chosen salt, so the server *must* keep passwords decryptable. That's an inherent Subsonic-protocol constraint; the mitigation is encryption at rest.

> [!WARNING]
> This server speaks plain HTTP. For anything beyond localhost, put it behind a reverse proxy (nginx / Caddy / Traefik) that terminates TLS. The server honors `X-Forwarded-Proto: https` when building absolute URLs.

## Building from source

```sh
git clone <repo-url>
cd tidal-subsonic
cargo build --release
cargo test                    # run the suite
./target/release/tidal-subsonic
```

**Requirements:** a recent stable **Rust** toolchain, a **C compiler** (`build-essential` / `gcc` / `clang`) for the bundled SQLite and the LAME MP3 encoder, and **Node + pnpm** for the embedded web portal (`build.rs` runs the frontend build). The `Dockerfile` and CI handle all of this for you.

## License

Licensed under the **GNU General Public License v3.0** — see [`LICENSE`](LICENSE) for the full text.

> Copyright © 2026 Soner Sayakci

This project talks to TIDAL's private APIs and is intended for personal use with your own TIDAL subscription; it is **not affiliated with or endorsed by TIDAL**.

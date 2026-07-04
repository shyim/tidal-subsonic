# tidal-subsonic

A [Subsonic](https://www.subsonic.org/pages/api.jsp)-compatible API server backed by [TIDAL](https://tidal.com). Point any Subsonic client at it and stream your TIDAL library — including HI-RES audio — through apps like Sublime Music, DSub, Ultrasonic, Submariner, or the Garmin YuMusic watch app.

It speaks the Subsonic REST API (both JSON and XML), talks to TIDAL's private APIs on the backend, and supports **multiple users** — each person links their own TIDAL account, and all tokens are encrypted at rest.

Written in Rust with [axum](https://github.com/tokio-rs/axum).

---

## Features

- **Full Subsonic REST API** — JSON and XML responses (`f=json` / `f=xml`), OpenSubsonic extensions advertised.
- **HI-RES streaming** — TIDAL HI-RES/lossless is delivered by reassembling DASH segments on the fly, with HTTP range/seek support so scrubbing works.
- **On-demand transcoding** — streams AAC by default; requesting `format=mp3` transparently transcodes to MP3 for clients that require it (e.g. the Garmin YuMusic app, which only declares MP3 support).
- **Disk media cache** — reassembled/transcoded audio is cached on disk so repeat plays don't re-fetch from TIDAL.
- **In-memory metadata cache** — per-user caching of browse/search/list responses for snappy navigation.
- **Cover art** — album/artist artwork proxied through `getCoverArt`.
- **Search** — `search2` / `search3` across artists, albums, and tracks.
- **Playlists** — list, view, **create, edit (add/remove tracks), and delete**.
- **Favorites / starring** — `star` / `unstar` mapped to TIDAL favorites.
- **Lyrics** — `getLyrics` and `getLyricsBySongId`.
- **Classic-client endpoints** — `getMusicDirectory`, `getIndexes`, `getOpenSubsonicExtensions`, `startScan`/`getScanStatus`, `getNowPlaying`, `scrobble`.
- **Multi-user** — admin-created accounts; each user links their own TIDAL account via a browser OAuth flow; per-user metadata isolation; `createUser` / `updateUser` / `deleteUser` / `changePassword`.
- **Security-hardened** — encrypted TIDAL tokens and Subsonic passwords at rest, POST-based credential entry for account linking, XSS / SSRF / path-traversal protections, log redaction, and a secure first-run admin bootstrap.

---

## Quick start

### Build and run

Build the **release** binary and run it. Do not use a debug build for real use — MP3 transcoding is roughly **4x slower** in debug.

```sh
cargo build --release
./target/release/tidal-subsonic
```

The server listens on **port 4533** by default. On first run it creates its database and prints a generated admin password to the log (see [First run](#first-run-and-admin) below).

> **Build requirement:** MP3 transcoding uses `mp3lame-encoder`, which compiles LAME from source via `cc`. You need a **C compiler** (`gcc`/`clang`, or `build-essential` on Debian/Ubuntu) available at build time. The bundled SQLite dependency also needs it.

Prefer not to build locally? See [Docker](#docker).

---

## Configuration

### Environment variables

| Variable | Purpose |
| --- | --- |
| `TIDAL_SUBSONIC_KEY` | Base64-encoded **32-byte** key used to encrypt TIDAL tokens (and Subsonic passwords) at rest. **Set this yourself** so the key is stable across restarts / container rebuilds. If unset, a key is generated on first run and stored in the database. |
| `TIDAL_SUBSONIC_ADMIN_PASSWORD` | Password for the `admin` user created on first run. If unset, a random password is generated and printed to the log **once**. |
| `RUST_LOG` | Standard `tracing` filter (e.g. `RUST_LOG=info` or `RUST_LOG=tidal_subsonic=debug`). |

Generate a stable encryption key:

```sh
head -c 32 /dev/urandom | base64
```

### Data locations

The database (users, encrypted tokens, config) lives in your OS config directory, and the on-disk media cache lives in your OS cache directory:

| OS | Database | Media cache |
| --- | --- | --- |
| Linux | `~/.config/tidal-subsonic/tidal-subsonic.db` | `~/.cache/tidal-subsonic/media/` |
| macOS | `~/Library/Application Support/tidal-subsonic/tidal-subsonic.db` | `~/Library/Caches/tidal-subsonic/media/` |

These directories are created automatically. When running in Docker, mount them as volumes to persist across container restarts (see [Docker](#docker)).

---

## First run and admin

On first launch the server bootstraps an **`admin`** user:

- If `TIDAL_SUBSONIC_ADMIN_PASSWORD` is set, that becomes the admin password.
- Otherwise a random password is generated and **printed to the log once**. Look for a line like:

  ```
  First run: created admin user 'admin' with generated password: <password>
  ```

  Copy it. To choose your own password up front, set `TIDAL_SUBSONIC_ADMIN_PASSWORD` before the first run. You can change it later with `changePassword`.

---

## Linking a TIDAL account

Each user links their own TIDAL account through a small built-in web page (this is separate from the Subsonic API):

1. Open **`http://<host>:4533/`** in a browser.
2. Enter your **Subsonic** username and password (this proves which account you're linking — the password is sent in the POST body, never the URL).
3. You'll be sent through TIDAL's OAuth login. After authorizing, the callback stores your encrypted TIDAL session against your Subsonic user.

Once linked, that user's Subsonic client can stream.

---

## Adding users

The `admin` user (or any admin) can create additional users through the Subsonic API. Each Subsonic endpoint takes standard Subsonic auth parameters (`u`, `p` / `t`+`s`, `c`, `v`, `f`).

Create a user:

```sh
curl "http://localhost:4533/rest/createUser?u=admin&p=<adminpass>&c=cli&v=1.16.1&f=json&username=alice&password=hunter2"
```

Then have that user open `http://<host>:4533/` to link their own TIDAL account. Manage users with `getUsers`, `updateUser`, `deleteUser`, and `changePassword`.

---

## Connecting a Subsonic client

Configure your client with:

- **Server / URL:** `http://<host>:4533`
- **Username / Password:** the Subsonic account credentials (admin or one you created)

Tested with Sublime Music, DSub, Ultrasonic, Submariner, and the Garmin YuMusic app.

### Client compatibility notes

- **Garmin YuMusic (and other MP3-only clients):** these declare only MP3 support. tidal-subsonic auto-handles this — when a client requests `format=mp3`, audio is transcoded to MP3 on the fly, so no extra configuration is needed.
- **Submariner:** has a quirk in how it edits playlists; playlist editing (`updatePlaylist`) is supported, but if a specific change doesn't take, re-fetch the playlist and retry.
- **Streaming quality:** HI-RES/lossless is served via DASH reassembly. Clients that support Subsonic transcoding negotiation still get MP3 when they ask for it.

---

## Supported endpoints

| Endpoint | Notes |
| --- | --- |
| `ping`, `getLicense` | System |
| `getMusicFolders`, `getIndexes`, `getGenres` | Browsing roots |
| `getArtists`, `getArtist`, `getAlbum`, `getSong` | Browsing |
| `getMusicDirectory` | Classic directory browsing |
| `getAlbumList`, `getAlbumList2`, `getRandomSongs` | Lists |
| `getStarred`, `getStarred2`, `star`, `unstar` | Favorites |
| `search2`, `search3` | Search |
| `getPlaylists`, `getPlaylist` | Playlist read |
| `createPlaylist`, `updatePlaylist`, `deletePlaylist` | Playlist write |
| `stream`, `download` | Audio (supports `format=mp3`, range/seek) |
| `getCoverArt`, `getAvatar` | Artwork |
| `getLyrics`, `getLyricsBySongId` | Lyrics |
| `scrobble`, `getNowPlaying` | Now playing |
| `getScanStatus`, `startScan` | Library scan |
| `getUser`, `getUsers` | User info |
| `createUser`, `updateUser`, `deleteUser`, `changePassword` | User management (admin) |
| `getOpenSubsonicExtensions` | OpenSubsonic capability advertisement |

Every endpoint is served at both `/rest/<name>` and `/rest/<name>.view`.

---

## Security notes

- **Encryption at rest:** TIDAL tokens and Subsonic passwords are encrypted with ChaCha20-Poly1305 using the `TIDAL_SUBSONIC_KEY` (or a DB-stored generated key). Set `TIDAL_SUBSONIC_KEY` yourself and keep it stable — losing it means every stored token/password becomes unreadable.
- **Account linking uses POST** so Subsonic passwords aren't placed in URLs, query strings, or logs.
- **SSRF protection:** outbound fetches (e.g. cover art) reject loopback, private, link-local, and unspecified addresses.
- **Path traversal:** cache keys are sanitized so nothing escapes the cache directory.
- **Log redaction:** credentials and tokens are kept out of logs.
- **Run behind TLS:** this server speaks plain HTTP. For anything beyond localhost, put it behind a reverse proxy (nginx/Caddy/Traefik) that terminates HTTPS. The server honors `X-Forwarded-Proto: https` when building absolute URLs.

---

## Building from source

```sh
git clone <repo-url>
cd tidal-subsonic
cargo build --release
cargo test        # run the test suite
./target/release/tidal-subsonic
```

Requirements:

- A recent stable **Rust** toolchain.
- A **C compiler** (`build-essential` / `gcc` / `clang`) — required to build the bundled SQLite and the LAME MP3 encoder.

---

## Docker

A multi-stage `Dockerfile` is included.

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

Notes:

- The image sets `XDG_CONFIG_HOME=/config` and `XDG_CACHE_HOME=/cache`, so the database lands in `/config/tidal-subsonic/` and the media cache in `/cache/tidal-subsonic/`. Mount both as volumes to persist data.
- Set `TIDAL_SUBSONIC_KEY` explicitly so the encryption key survives image/container rebuilds. Without it, a new key is generated inside the container's `/config` volume.
- If you don't set `TIDAL_SUBSONIC_ADMIN_PASSWORD`, check the container logs (`docker logs tidal-subsonic`) for the generated admin password.

---

## License

GPL-3.0

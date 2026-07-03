# tidal-subsonic

A Subsonic-compatible API server backed by TIDAL. Stream your TIDAL library through any Subsonic client (Sublime Music, DSub, Ultrasonic, etc.).

## Setup

### 1. Get TIDAL API credentials

You need a TIDAL Client ID. The server supports the Device Code OAuth flow.

Set your Client ID in the config file:

```toml
# ~/.config/tidal-subsonic/config.toml
[tidal]
client_id = "your_client_id"
max_quality = "HI_RES_LOSSLESS"  # or "LOSSLESS", "HIGH", "LOW"
```

### 2. Authenticate with TIDAL

Run the server once after setting your client_id. It will auto-refresh tokens using the refresh token stored in config.

### 3. Connect your Subsonic client

Default credentials:
- **Server URL**: `http://your-server:4533`
- **Username**: `tidal`
- **Password**: `tidal`

You can change these in the config file under `[subsonic]`.

## Supported Subsonic API Endpoints

### System
- `ping` ✅
- `getLicense` ✅

### Browsing
- `getMusicFolders` ✅
- `getIndexes` ✅ (favorite artists)
- `getArtists` ✅ (favorite artists)
- `getArtist` ✅ (with albums)
- `getAlbum` ✅ (with track listing)
- `getSong` ✅

### Album/Song Lists
- `getAlbumList` ✅ (from favorites)
- `getAlbumList2` ✅
- `getRandomSongs` ✅

### Searching
- `search2` ✅
- `search3` ✅

### Playlists
- `getPlaylists` ✅
- `getPlaylist` ✅
- `createPlaylist` ❌ (read-only)
- `updatePlaylist` ❌ (read-only)
- `deletePlaylist` ❌ (read-only)

### Media
- `stream` ✅ (proxies TIDAL audio; reconstructs segmented DASH — incl. HI-RES lossless — into a single MP4, and forwards HTTP range requests for seeking)
- `getCoverArt` ✅ (size-aware; falls back across TIDAL's album/artist image sizes)

### User
- `getUser` ✅
- `getAvatar` ✅ (placeholder)

### Other
- `getStarred` / `getStarred2` ✅
- `star` / `unstar` ✅ (adds/removes TIDAL favorites for songs, albums, and artists)
- `getScanStatus` ✅
- `getGenres` ✅ (empty)
- `scrobble` ✅ (acknowledged, not forwarded)

## Response formats

Both XML and JSON are supported. Clients that pass `f=json` (Symfonium, Feishin,
Amperfy, Tempo, …) get JSON; `f=jsonp` with a `callback` param is also handled.
Everything else defaults to XML.

## Authentication

Subsonic token auth (`t` = md5(password + salt) with `s`) and the legacy
plaintext/`enc:`-hex `p` password parameter are both accepted.

## Building

```bash
cargo build --release
./target/release/tidal-subsonic
```

## License

GPL-3.0

use crate::item_id::ItemId;
use crate::subsonic::*;
use crate::tidal_client::*;

/// Extract the primary artist name and TIDAL id from an album detail, preferring
/// the singular `artist` field and falling back to the first entry of `artists`.
/// Returns the display name (or "Unknown Artist") and the numeric artist id if
/// one is present.
pub fn primary_artist(album: &TidalAlbumDetail) -> (String, Option<u64>) {
    let primary = album
        .artist
        .as_ref()
        .or_else(|| album.artists.as_ref().and_then(|a| a.first()));
    let name = primary
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let id = primary.map(|a| a.id);
    (name, id)
}

/// Build a Subsonic cover-art ID from a Tidal image reference. TIDAL image
/// references are UUIDs like `b66a5c40-c34d-4507-a0dc-5f98e46fdd20`; we store
/// the raw reference (base64-encoded) and reconstruct the CDN URL on fetch.
pub fn cover_art_id(image_ref: &str) -> String {
    format!("ca-{}", base64_encode(image_ref))
}

/// Candidate CDN URLs for a cover-art ID, ordered by how well they match the
/// requested square size. TIDAL serves different size sets for different image
/// kinds (albums: 80/160/320/640/1280; artists: 160/320/480/750), so the caller
/// tries each in order until one returns 200. In a TIDAL image UUID the dashes
/// are path separators, e.g. `b66a5c40-c34d-...` →
/// `.../images/b66a5c40/c34d/.../{size}x{size}.jpg`.
pub fn cover_art_urls(cover_id: &str, size: u32) -> Vec<String> {
    let raw = if let Some(encoded) = cover_id.strip_prefix("ca-") {
        match base64_decode(encoded) {
            Some(r) => r,
            None => return vec![],
        }
    } else if let Some(uuid) = cover_id.strip_prefix("cover-") {
        uuid.to_string()
    } else {
        return vec![];
    };

    // If a full URL was stored, use it directly.
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return vec![raw];
    }

    let path = raw.trim_start_matches('/').replace('-', "/");
    // Union of the sizes TIDAL serves across image kinds, ordered so the ones
    // closest to (and not smaller than) the request come first.
    const SIZES: [u32; 8] = [80, 160, 320, 480, 640, 750, 1080, 1280];
    let want = if size == 0 { 640 } else { size };
    let mut ordered: Vec<u32> = SIZES.to_vec();
    ordered.sort_by_key(|&s| (s < want, (s as i64 - want as i64).abs()));
    ordered
        .into_iter()
        .map(|dim| {
            format!(
                "https://resources.tidal.com/images/{}/{}x{}.jpg",
                path, dim, dim
            )
        })
        .collect()
}

fn base64_encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes())
}

fn base64_decode(s: &str) -> Option<String> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_art_id_roundtrips_dashes_to_slashes() {
        let raw = "b66a5c40-c34d-4507-a0dc-5f98e46fdd20";
        let id = cover_art_id(raw);
        let url = &cover_art_urls(&id, 640)[0];
        assert_eq!(
            url,
            "https://resources.tidal.com/images/b66a5c40/c34d/4507/a0dc/5f98e46fdd20/640x640.jpg"
        );
    }

    #[test]
    fn cover_art_urls_prefers_requested_size_then_falls_back() {
        let id = cover_art_id("aa-bb");
        let urls = cover_art_urls(&id, 320);
        // First candidate is the exact requested size.
        assert!(urls[0].contains("/320x320.jpg"));
        // Every serveable size is offered as a fallback.
        assert_eq!(urls.len(), 8);
        // A size TIDAL doesn't serve for one kind is still reachable via fallback.
        assert!(urls.iter().any(|u| u.contains("/480x480.jpg")));
    }

    #[test]
    fn cover_art_urls_default_size_is_640() {
        let id = cover_art_id("aa-bb");
        assert!(cover_art_urls(&id, 0)[0].contains("/640x640.jpg"));
    }

    #[test]
    fn cover_art_urls_passes_through_full_urls() {
        let id = cover_art_id("https://example.com/a.jpg");
        assert_eq!(cover_art_urls(&id, 640), vec!["https://example.com/a.jpg"]);
    }
}

/// Convert a Tidal track to a Subsonic Child (song entry)
pub fn track_to_child(track: &TidalTrack, _base_url: &str) -> SubsonicChild {
    let album_id = track.album.as_ref().map(|a| ItemId::Album(a.id).to_string());
    let cover_art = track
        .album
        .as_ref()
        .and_then(|a| a.cover.as_ref())
        .map(|c| cover_art_id(c));
    let primary = track
        .artist
        .as_ref()
        .or_else(|| track.artists.as_ref().and_then(|a| a.first()));
    let artist_name = primary
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let artist_id = primary.map(|a| ItemId::Artist(a.id).to_string());

    // TIDAL delivers every quality as fragmented MP4 (AAC or FLAC-in-MP4), and
    // the proxy concatenates those segments into an MP4 file — so advertise the
    // container the client will actually receive, not the inner codec.
    let suffix = "m4a";
    let content_type = "audio/mp4";

    SubsonicChild {
        id: ItemId::Track(track.id).to_string(),
        parent: album_id.clone(),
        is_dir: false,
        title: track.title.clone(),
        album: track.album.as_ref().map(|a| a.title.clone()),
        artist: Some(artist_name),
        track: track.track_number,
        year: track
            .album
            .as_ref()
            .and_then(|a| a.release_date.as_ref())
            .and_then(|d| d[..4].parse::<u32>().ok()),
        genre: None,
        cover_art,
        size: None, // Size is unknown until we stream
        content_type: Some(content_type.to_string()),
        suffix: Some(suffix.to_string()),
        transcoded_content_type: Some(content_type.to_string()),
        transcoded_suffix: Some(suffix.to_string()),
        duration: Some(track.duration),
        bit_rate: None,
        path: Some(ItemId::Track(track.id).to_string()),
        is_video: Some(false),
        play_count: None,
        disc_number: track.volume_number.or(Some(1)),
        created: None,
        starred: None,
        album_id,
        artist_id,
        media_type: Some("music".to_string()),
        bookmark_position: None,
    }
}

/// Convert a Tidal artist to Subsonic Artist
pub fn artist_to_subsonic(artist: &TidalArtistDetail) -> SubsonicArtist {
    SubsonicArtist {
        id: ItemId::Artist(artist.id).to_string(),
        name: artist.name.clone(),
        cover_art: artist
            .picture
            .as_ref()
            .map(|p| cover_art_id(p)),
        album_count: None,
        artist_image_url: artist.picture.clone(),
        starred: None,
        user_rating: None,
    }
}

/// Convert a search TidalArtist to Subsonic Artist
pub fn search_artist_to_subsonic(artist: &TidalArtist) -> SubsonicArtist {
    SubsonicArtist {
        id: ItemId::Artist(artist.id).to_string(),
        name: artist.name.clone(),
        cover_art: artist
            .picture
            .as_ref()
            .map(|p| cover_art_id(p)),
        album_count: None,
        artist_image_url: artist.picture.clone(),
        starred: None,
        user_rating: None,
    }
}

/// Convert a Tidal album to Subsonic Album
pub fn album_to_subsonic(album: &TidalAlbumDetail) -> SubsonicAlbum {
    let (artist_name, artist_id) = primary_artist(album);
    let artist_id = artist_id.map(|id| ItemId::Artist(id).to_string());

    let year = album
        .release_date
        .as_ref()
        .and_then(|d| d[..4].parse::<u32>().ok());

    SubsonicAlbum {
        id: ItemId::Album(album.id).to_string(),
        name: if let Some(ref version) = album.version {
            if !version.is_empty() {
                format!("{} ({})", album.title, version)
            } else {
                album.title.clone()
            }
        } else {
            album.title.clone()
        },
        artist: Some(artist_name),
        artist_id,
        cover_art: album
            .cover
            .as_ref()
            .map(|c| cover_art_id(c)),
        song_count: album.number_of_tracks,
        duration: album.duration,
        created: album.release_date.clone(),
        year,
        genre: None,
        starred: None,
    }
}

/// Represent a Tidal album as a directory child (a folder) for
/// `getMusicDirectory`, where an artist's children are its albums.
pub fn album_to_directory_child(album: &TidalAlbumDetail, artist_id: &str) -> SubsonicChild {
    let a = album_to_subsonic(album);
    SubsonicChild {
        id: a.id.clone(),
        parent: Some(artist_id.to_string()),
        is_dir: true,
        title: a.name,
        album: None,
        artist: a.artist,
        track: None,
        year: a.year,
        genre: None,
        cover_art: a.cover_art,
        size: None,
        content_type: None,
        suffix: None,
        transcoded_content_type: None,
        transcoded_suffix: None,
        duration: a.duration,
        bit_rate: None,
        path: None,
        is_video: Some(false),
        play_count: None,
        disc_number: None,
        created: a.created,
        starred: None,
        album_id: Some(a.id),
        artist_id: a.artist_id,
        media_type: None,
        bookmark_position: None,
    }
}

pub fn album_detail_to_album_with_songs(
    album: &TidalAlbumDetail,
    tracks: &[TidalTrack],
    base_url: &str,
) -> AlbumWithSongs {
    let (artist_name, artist_id) = primary_artist(album);
    let artist_id = artist_id.map(|id| ItemId::Artist(id).to_string());
    let year = album
        .release_date
        .as_ref()
        .and_then(|d| d[..4].parse::<u32>().ok());

    AlbumWithSongs {
        id: ItemId::Album(album.id).to_string(),
        name: album.title.clone(),
        artist: Some(artist_name),
        artist_id,
        cover_art: album
            .cover
            .as_ref()
            .map(|c| cover_art_id(c)),
        song_count: Some(tracks.len() as u32),
        duration: album.duration,
        year,
        genre: None,
        song: tracks.iter().map(|t| track_to_child(t, base_url)).collect(),
    }
}

/// Convert a Tidal playlist to Subsonic Playlist
pub fn playlist_to_subsonic(playlist: &TidalPlaylist) -> SubsonicPlaylist {
    SubsonicPlaylist {
        id: ItemId::Playlist(playlist.uuid.clone()).to_string(),
        name: playlist.title.clone(),
        comment: playlist.description.clone(),
        owner: playlist
            .creator
            .as_ref()
            .and_then(|c| c.name.clone()),
        public: playlist.public_playlist,
        song_count: playlist.number_of_tracks,
        duration: playlist.duration,
        created: playlist.created.clone(),
        changed: playlist.last_updated.clone(),
        cover_art: playlist
            .square_image
            .as_ref()
            .or(playlist.image.as_ref())
            .map(|i| cover_art_id(i)),
    }
}

pub fn build_indexes(artists: &[TidalArtistDetail]) -> Indexes {
    let mut indexes: std::collections::BTreeMap<char, Vec<SubsonicArtist>> =
        std::collections::BTreeMap::new();

    for artist in artists {
        let first_char = artist
            .name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .next()
            .unwrap_or('?');
        let key = if first_char.is_alphabetic() {
            first_char
        } else {
            '#'
        };
        indexes.entry(key).or_default().push(artist_to_subsonic(artist));
    }

    let index_vec: Vec<Index> = indexes
        .into_iter()
        .map(|(name, artists)| Index {
            name: name.to_string(),
            artist: artists,
        })
        .collect();

    Indexes {
        last_modified: 0,
        ignored_articles: "The El La Los Las Le Les".to_string(),
        index: index_vec,
        child: None,
    }
}

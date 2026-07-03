use crate::subsonic::*;
use crate::tidal_client::*;

/// Convert a Tidal track to a Subsonic Child (song entry)
pub fn track_to_child(track: &TidalTrack, _base_url: &str) -> SubsonicChild {
    let album_id = track.album.as_ref().map(|a| format!("al-{}", a.id));
    let cover_art = track
        .album
        .as_ref()
        .and_then(|a| a.cover.as_ref())
        .map(|c| format!("cover-{}", c.replace('/', "_").replace(':', "_")));
    let artist_name = track
        .artist
        .as_ref()
        .map(|a| a.name.clone())
        .or_else(|| track.artists.as_ref().and_then(|a| a.first()).map(|a| a.name.clone()))
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let artist_id = track
        .artist
        .as_ref()
        .map(|a| format!("ar-{}", a.id))
        .or_else(|| {
            track
                .artists
                .as_ref()
                .and_then(|a| a.first())
                .map(|a| format!("ar-{}", a.id))
        });

    // Determine suffix based on audio quality
    let suffix = track
        .audio_quality
        .as_ref()
        .map(|q| match q.as_str() {
            "LOW" | "HIGH" => "m4a",
            "LOSSLESS" | "HI_RES" | "HI_RES_LOSSLESS" => "flac",
            _ => "flac",
        })
        .unwrap_or("flac");

    let content_type = match suffix {
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        _ => "audio/mpeg",
    };

    SubsonicChild {
        id: format!("tr-{}", track.id),
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
        path: Some(format!("tr-{}", track.id)),
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
        id: format!("ar-{}", artist.id),
        name: artist.name.clone(),
        cover_art: artist
            .picture
            .as_ref()
            .map(|p| format!("cover-{}", p.replace('/', "_").replace(':', "_"))),
        album_count: None,
        artist_image_url: artist.picture.clone(),
        starred: None,
        user_rating: None,
    }
}

/// Convert a search TidalArtist to Subsonic Artist
pub fn search_artist_to_subsonic(artist: &TidalArtist) -> SubsonicArtist {
    SubsonicArtist {
        id: format!("ar-{}", artist.id),
        name: artist.name.clone(),
        cover_art: artist
            .picture
            .as_ref()
            .map(|p| format!("cover-{}", p.replace('/', "_").replace(':', "_"))),
        album_count: None,
        artist_image_url: artist.picture.clone(),
        starred: None,
        user_rating: None,
    }
}

/// Convert a Tidal album to Subsonic Album
pub fn album_to_subsonic(album: &TidalAlbumDetail) -> SubsonicAlbum {
    let artist_name = album
        .artist
        .as_ref()
        .map(|a| a.name.clone())
        .or_else(|| album.artists.as_ref().and_then(|a| a.first()).map(|a| a.name.clone()))
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let artist_id = album
        .artist
        .as_ref()
        .map(|a| format!("ar-{}", a.id))
        .or_else(|| {
            album
                .artists
                .as_ref()
                .and_then(|a| a.first())
                .map(|a| format!("ar-{}", a.id))
        });

    let year = album
        .release_date
        .as_ref()
        .and_then(|d| d[..4].parse::<u32>().ok());

    SubsonicAlbum {
        id: format!("al-{}", album.id),
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
            .map(|c| format!("cover-{}", c.replace('/', "_").replace(':', "_"))),
        song_count: album.number_of_tracks,
        duration: album.duration,
        created: album.release_date.clone(),
        year,
        genre: None,
        starred: None,
    }
}

pub fn album_detail_to_album_with_songs(
    album: &TidalAlbumDetail,
    tracks: &[TidalTrack],
    base_url: &str,
) -> AlbumWithSongs {
    let artist_name = album
        .artist
        .as_ref()
        .map(|a| a.name.clone())
        .or_else(|| album.artists.as_ref().and_then(|a| a.first()).map(|a| a.name.clone()))
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let artist_id = album
        .artist
        .as_ref()
        .map(|a| format!("ar-{}", a.id))
        .or_else(|| {
            album
                .artists
                .as_ref()
                .and_then(|a| a.first())
                .map(|a| format!("ar-{}", a.id))
        });
    let year = album
        .release_date
        .as_ref()
        .and_then(|d| d[..4].parse::<u32>().ok());

    AlbumWithSongs {
        id: format!("al-{}", album.id),
        name: album.title.clone(),
        artist: Some(artist_name),
        artist_id,
        cover_art: album
            .cover
            .as_ref()
            .map(|c| format!("cover-{}", c.replace('/', "_").replace(':', "_"))),
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
        id: format!("pl-{}", playlist.uuid),
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
            .map(|i| format!("cover-{}", i.replace('/', "_").replace(':', "_"))),
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

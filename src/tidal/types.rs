use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TidalTrack {
    pub id: u64,
    pub title: String,
    pub duration: u32,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub artist: Option<TidalArtist>,
    #[serde(default)]
    pub artists: Option<Vec<TidalArtist>>,
    #[serde(default)]
    pub album: Option<TidalAlbum>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub volume_number: Option<u32>,
    #[serde(default)]
    pub isrc: Option<String>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(default)]
    pub replay_gain: Option<f64>,
    #[serde(default)]
    pub peak: Option<f64>,
    #[serde(default)]
    pub copyright: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub stream_ready: Option<bool>,
    #[serde(default)]
    pub allow_streaming: Option<bool>,
    #[serde(default)]
    pub stream_start_date: Option<String>,
    #[serde(default)]
    pub audio_modes: Option<Vec<String>>,
    #[serde(default)]
    pub media_metadata: Option<MediaMetadata>,
    #[serde(default)]
    pub mixes: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalArtist {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default, rename = "type")]
    pub artist_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalArtistDetail {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalAlbum {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub vibrant_color: Option<String>,
    #[serde(default)]
    pub video_cover: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalAlbumDetail {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub vibrant_color: Option<String>,
    #[serde(default)]
    pub video_cover: Option<String>,
    #[serde(default)]
    pub artist: Option<TidalArtist>,
    #[serde(default)]
    pub artists: Option<Vec<TidalArtist>>,
    #[serde(default)]
    pub number_of_tracks: Option<u32>,
    #[serde(default)]
    pub number_of_videos: Option<u32>,
    #[serde(default)]
    pub number_of_volumes: Option<u32>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub upc: Option<String>,
    #[serde(default, rename = "type")]
    pub album_type: Option<String>,
    #[serde(default)]
    pub copyright: Option<String>,
    #[serde(default)]
    pub explicit: Option<bool>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub stream_ready: Option<bool>,
    #[serde(default)]
    pub allow_streaming: Option<bool>,
    #[serde(default)]
    pub stream_start_date: Option<String>,
    #[serde(default)]
    pub audio_modes: Option<Vec<String>>,
    #[serde(default)]
    pub media_metadata: Option<MediaMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalPlaylist {
    pub uuid: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub square_image: Option<String>,
    #[serde(default)]
    pub number_of_tracks: Option<u32>,
    #[serde(default)]
    pub creator: Option<TidalPlaylistCreator>,
    #[serde(default, rename = "type")]
    pub playlist_type: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub last_updated: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub public_playlist: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalPlaylistCreator {
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total_number_of_items: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedTracks {
    pub items: Vec<TidalTrack>,
    pub total_number_of_items: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TidalSearchResults {
    pub artists: Vec<TidalArtist>,
    pub albums: Vec<TidalAlbumDetail>,
    pub tracks: Vec<TidalTrack>,
    pub playlists: Vec<TidalPlaylist>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub url: String,
    /// For segmented DASH streams: the ordered list of segment URLs (init
    /// segment first) that must be concatenated to reconstruct a playable file.
    /// Empty when `url` is a single directly-downloadable file.
    #[serde(default)]
    pub segments: Vec<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub bit_depth: Option<u32>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub manifest: Option<String>,
    #[serde(default)]
    pub manifest_mime_type: Option<String>,
    #[serde(default)]
    pub album_replay_gain: Option<f64>,
    #[serde(default)]
    pub album_peak_amplitude: Option<f64>,
    #[serde(default)]
    pub track_replay_gain: Option<f64>,
    #[serde(default)]
    pub track_peak_amplitude: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPageResponse {
    pub album: TidalAlbumDetail,
    pub tracks: Vec<TidalTrack>,
    pub total_tracks: u32,
    pub vibrant_color: Option<String>,
    pub copyright: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TidalLyrics {
    #[serde(default)]
    pub lyrics: Option<String>,
    #[serde(default)]
    pub subtitles: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
}

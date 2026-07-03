use serde::{Deserialize, Serialize};

/// Root Subsonic response element
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "subsonic-response")]
pub struct SubsonicResponse {
    #[serde(rename = "@xmlns")]
    pub xmlns: String,
    #[serde(rename = "@status")]
    pub status: String,
    #[serde(rename = "@version")]
    pub version: String,
    #[serde(rename = "@type")]
    pub server_type: String,
    #[serde(rename = "@serverVersion")]
    pub server_version: String,
    #[serde(rename = "@openSubsonic")]
    pub open_subsonic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SubsonicError>,
    // One of the following may be present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "musicFolders")]
    pub music_folders: Option<MusicFolders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexes: Option<Indexes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artists: Option<ArtistsList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<ArtistWithAlbums>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<AlbumWithSongs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<SubsonicSong>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "albumList")]
    pub album_list: Option<AlbumList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "albumList2")]
    pub album_list2: Option<AlbumList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "randomSongs")]
    pub random_songs: Option<SongList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "songsByGenre")]
    pub songs_by_genre: Option<SongList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nowPlaying")]
    pub now_playing: Option<NowPlaying>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starred: Option<Starred>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starred2: Option<Starred2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "searchResult")]
    pub search_result: Option<SearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "searchResult2")]
    pub search_result2: Option<SearchResult2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "searchResult3")]
    pub search_result3: Option<SearchResult3>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlists: Option<PlaylistsWrapper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<PlaylistWithSongs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "user")]
    pub user: Option<SubsonicUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "scanStatus")]
    pub scan_status: Option<ScanStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "genres")]
    pub genres: Option<GenresWrapper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "openSubsonicExtensions")]
    pub open_subsonic_extensions: Option<Vec<OpenSubsonicExtension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "directory")]
    pub directory: Option<Directory>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Directory {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@parent")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@playCount")]
    pub play_count: Option<u64>,
    #[serde(rename = "child", default)]
    pub child: Vec<SubsonicChild>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OpenSubsonicExtension {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "versions")]
    pub versions: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicError {
    #[serde(rename = "@code")]
    pub code: u32,
    #[serde(rename = "@message")]
    pub message: String,
}

// ------ System ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct License {
    #[serde(rename = "@valid")]
    pub valid: bool,
    #[serde(rename = "@email", skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(rename = "@licenseExpires", skip_serializing_if = "Option::is_none")]
    pub license_expires: Option<String>,
    #[serde(rename = "@trialExpires", skip_serializing_if = "Option::is_none")]
    pub trial_expires: Option<String>,
}

// ------ Browsing ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MusicFolders {
    #[serde(rename = "musicFolder", default)]
    pub music_folder: Vec<MusicFolder>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MusicFolder {
    #[serde(rename = "@id")]
    pub id: u32,
    #[serde(rename = "@name")]
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Indexes {
    #[serde(rename = "@lastModified")]
    pub last_modified: u64,
    #[serde(rename = "@ignoredArticles")]
    pub ignored_articles: String,
    #[serde(rename = "index", default)]
    pub index: Vec<Index>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "child", default)]
    pub child: Option<Vec<SubsonicChild>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Index {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "artist", default)]
    pub artist: Vec<SubsonicArtist>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicArtist {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@albumCount")]
    pub album_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artistImageUrl")]
    pub artist_image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@starred")]
    pub starred: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@userRating")]
    pub user_rating: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtistsList {
    #[serde(rename = "@ignoredArticles")]
    pub ignored_articles: String,
    #[serde(rename = "index", default)]
    pub index: Vec<Index>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtistWithAlbums {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@albumCount")]
    pub album_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artistImageUrl")]
    pub artist_image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@starred")]
    pub starred: Option<String>,
    #[serde(rename = "album", default)]
    pub album: Vec<SubsonicAlbum>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicAlbum {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artist")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artistId")]
    pub artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@songCount")]
    pub song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@duration")]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@created")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@year")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@genre")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@starred")]
    pub starred: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlbumWithSongs {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artist")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artistId")]
    pub artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@songCount")]
    pub song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@duration")]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@year")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@genre")]
    pub genre: Option<String>,
    #[serde(rename = "song", default)]
    pub song: Vec<SubsonicChild>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicChild {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@parent")]
    pub parent: Option<String>,
    #[serde(rename = "@isDir")]
    pub is_dir: bool,
    #[serde(rename = "@title")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@album")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artist")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@track")]
    pub track: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@year")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@genre")]
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@size")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@contentType")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@suffix")]
    pub suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@transcodedContentType")]
    pub transcoded_content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@transcodedSuffix")]
    pub transcoded_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@duration")]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@bitRate")]
    pub bit_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@path")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@isVideo")]
    pub is_video: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@playCount")]
    pub play_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@discNumber")]
    pub disc_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@created")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@starred")]
    pub starred: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@albumId")]
    pub album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@artistId")]
    pub artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@type")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@bookmarkPosition")]
    pub bookmark_position: Option<u64>,
}

// Alias for songs in song lists
pub type SubsonicSong = SubsonicChild;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SongList {
    #[serde(rename = "song", default)]
    pub song: Vec<SubsonicChild>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlbumList {
    #[serde(rename = "album", default)]
    pub album: Vec<SubsonicAlbum>,
}

// ------ Starred ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Starred {
    #[serde(rename = "artist", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<Vec<SubsonicArtist>>,
    #[serde(rename = "album", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<Vec<SubsonicAlbum>>,
    #[serde(rename = "song", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<Vec<SubsonicChild>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Starred2 {
    #[serde(rename = "artist", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<Vec<SubsonicArtist>>,
    #[serde(rename = "album", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<Vec<SubsonicAlbum>>,
    #[serde(rename = "song", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<Vec<SubsonicChild>>,
}

// ------ Search ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    #[serde(rename = "artist", default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<Vec<SubsonicArtist>>,
    #[serde(rename = "album", default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Vec<SubsonicAlbum>>,
    #[serde(rename = "song", default, skip_serializing_if = "Option::is_none")]
    pub song: Option<Vec<SubsonicChild>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult2 {
    #[serde(rename = "artist", default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<Vec<SubsonicArtist>>,
    #[serde(rename = "album", default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Vec<SubsonicAlbum>>,
    #[serde(rename = "song", default, skip_serializing_if = "Option::is_none")]
    pub song: Option<Vec<SubsonicChild>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult3 {
    #[serde(rename = "artist", default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<Vec<SubsonicArtist>>,
    #[serde(rename = "album", default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Vec<SubsonicAlbum>>,
    #[serde(rename = "song", default, skip_serializing_if = "Option::is_none")]
    pub song: Option<Vec<SubsonicChild>>,
}

// ------ Playlists ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlaylistsWrapper {
    #[serde(rename = "playlist", default)]
    pub playlist: Vec<SubsonicPlaylist>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicPlaylist {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@comment")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@owner")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@public")]
    pub public: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@songCount")]
    pub song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@duration")]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@created")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@changed")]
    pub changed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlaylistWithSongs {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@comment")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@owner")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@public")]
    pub public: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@songCount")]
    pub song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@duration")]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@created")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@changed")]
    pub changed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArt")]
    pub cover_art: Option<String>,
    #[serde(rename = "entry", default)]
    pub entry: Vec<SubsonicChild>,
}

// ------ User ------ 

#[derive(Debug, Serialize, Deserialize)]
pub struct SubsonicUser {
    #[serde(rename = "@username")]
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@email")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@scrobblingEnabled")]
    pub scrobbling_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@adminRole")]
    pub admin_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@settingsRole")]
    pub settings_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@downloadRole")]
    pub download_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@uploadRole")]
    pub upload_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@playlistRole")]
    pub playlist_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@coverArtRole")]
    pub cover_art_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@commentRole")]
    pub comment_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@podcastRole")]
    pub podcast_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@streamRole")]
    pub stream_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@jukeboxRole")]
    pub jukebox_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@shareRole")]
    pub share_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@videoConversionRole")]
    pub video_conversion_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@avatarLastChanged")]
    pub avatar_last_changed: Option<String>,
    #[serde(rename = "folder", default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<Vec<u32>>,
}

// ------ Scan status ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScanStatus {
    #[serde(rename = "@scanning")]
    pub scanning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "@count")]
    pub count: Option<u64>,
}

// ------ Now Playing ------ 

#[derive(Debug, Serialize, Deserialize)]
pub struct NowPlaying {
    #[serde(rename = "entry", default)]
    pub entry: Vec<NowPlayingEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NowPlayingEntry {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@username")]
    pub username: String,
    #[serde(rename = "@minutesAgo")]
    pub minutes_ago: u32,
    #[serde(rename = "@playerId")]
    pub player_id: u32,
    #[serde(rename = "@playerName")]
    pub player_name: String,
}

// ------ Genres ------ 

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GenresWrapper {
    #[serde(rename = "genre", default)]
    pub genre: Vec<SubsonicGenre>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicGenre {
    #[serde(rename = "@songCount")]
    pub song_count: u64,
    #[serde(rename = "@albumCount")]
    pub album_count: u64,
    #[serde(rename = "$value")]
    pub value: String,
}

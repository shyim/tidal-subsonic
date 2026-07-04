use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};

/// The single payload an endpoint attaches to a response. Each variant maps to
/// exactly one Subsonic wire element/key (e.g. `albumList`, `searchResult3`).
/// The response's manual `Serialize` impl emits the variant under that exact
/// name, reproducing the historical per-field output byte-for-byte.
#[derive(Debug)]
pub enum Payload {
    License(License),
    MusicFolders(MusicFolders),
    Indexes(Indexes),
    Artists(ArtistsList),
    Artist(ArtistWithAlbums),
    Album(AlbumWithSongs),
    Song(SubsonicSong),
    AlbumList(AlbumList),
    AlbumList2(AlbumList),
    RandomSongs(SongList),
    NowPlaying(NowPlaying),
    /// getStarred emits both `<starred>` and `<starred2>` in one response.
    Starred(Starred, Starred2),
    SearchResult2(SearchResult2),
    SearchResult3(SearchResult3),
    Playlists(PlaylistsWrapper),
    Playlist(PlaylistWithSongs),
    User(SubsonicUser),
    Users(Users),
    ScanStatus(ScanStatus),
    Genres(GenresWrapper),
    OpenSubsonicExtensions(Vec<OpenSubsonicExtension>),
    Directory(Directory),
}

impl Payload {
    /// Serialize the payload as one (or, for `Starred`, two) named field(s) of
    /// the parent response struct, using the exact historical element/key names.
    fn serialize_into<S: SerializeStruct>(&self, st: &mut S) -> Result<(), S::Error> {
        match self {
            Payload::License(v) => st.serialize_field("license", v),
            Payload::MusicFolders(v) => st.serialize_field("musicFolders", v),
            Payload::Indexes(v) => st.serialize_field("indexes", v),
            Payload::Artists(v) => st.serialize_field("artists", v),
            Payload::Artist(v) => st.serialize_field("artist", v),
            Payload::Album(v) => st.serialize_field("album", v),
            Payload::Song(v) => st.serialize_field("song", v),
            Payload::AlbumList(v) => st.serialize_field("albumList", v),
            Payload::AlbumList2(v) => st.serialize_field("albumList2", v),
            Payload::RandomSongs(v) => st.serialize_field("randomSongs", v),
            Payload::NowPlaying(v) => st.serialize_field("nowPlaying", v),
            Payload::Starred(s, s2) => {
                st.serialize_field("starred", s)?;
                st.serialize_field("starred2", s2)
            }
            Payload::SearchResult2(v) => st.serialize_field("searchResult2", v),
            Payload::SearchResult3(v) => st.serialize_field("searchResult3", v),
            Payload::Playlists(v) => st.serialize_field("playlists", v),
            Payload::Playlist(v) => st.serialize_field("playlist", v),
            Payload::User(v) => st.serialize_field("user", v),
            Payload::Users(v) => st.serialize_field("users", v),
            Payload::ScanStatus(v) => st.serialize_field("scanStatus", v),
            Payload::Genres(v) => st.serialize_field("genres", v),
            Payload::OpenSubsonicExtensions(v) => st.serialize_field("openSubsonicExtensions", v),
            Payload::Directory(v) => st.serialize_field("directory", v),
        }
    }

    /// Number of struct fields this payload contributes (2 only for `Starred`).
    fn field_count(&self) -> usize {
        match self {
            Payload::Starred(_, _) => 2,
            _ => 1,
        }
    }
}

/// Root Subsonic response element. Carries the six fixed metadata attributes,
/// an optional error, and at most one payload. A hand-written `Serialize` impl
/// emits the payload under its historical element/key name (quick-xml cannot
/// `#[serde(flatten)]` an enum without losing the root tag).
#[derive(Debug)]
pub struct SubsonicResponse {
    pub xmlns: String,
    pub status: String,
    pub version: String,
    pub server_type: String,
    pub server_version: String,
    pub open_subsonic: bool,
    pub error: Option<SubsonicError>,
    pub payload: Option<Payload>,
}

const XMLNS: &str = "http://subsonic.org/restapi";
const API_VERSION: &str = "1.16.1";
const SERVER_NAME: &str = "tidal-subsonic";

impl SubsonicResponse {
    fn base(status: &str) -> Self {
        SubsonicResponse {
            xmlns: XMLNS.to_string(),
            status: status.to_string(),
            version: API_VERSION.to_string(),
            server_type: SERVER_NAME.to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            open_subsonic: true,
            error: None,
            payload: None,
        }
    }

    /// A successful response with no payload (e.g. ping, scrobble).
    pub fn ok() -> Self {
        Self::base("ok")
    }

    /// A successful response carrying a single payload.
    pub fn ok_with(payload: Payload) -> Self {
        let mut r = Self::base("ok");
        r.payload = Some(payload);
        r
    }

    /// A failed response carrying a Subsonic error code and message.
    pub fn error(code: u32, message: &str) -> Self {
        let mut r = Self::base("failed");
        r.error = Some(SubsonicError {
            code,
            message: message.to_string(),
        });
        r
    }
}

impl Serialize for SubsonicResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Fixed six attributes + optional error + payload field(s). The field
        // count must be exact for quick-xml. `@`-prefixed names become XML
        // attributes; unprefixed become child elements (JSON strips the `@`).
        let mut n = 6;
        if self.error.is_some() {
            n += 1;
        }
        if let Some(p) = &self.payload {
            n += p.field_count();
        }
        let mut st = serializer.serialize_struct("subsonic-response", n)?;
        st.serialize_field("@xmlns", &self.xmlns)?;
        st.serialize_field("@status", &self.status)?;
        st.serialize_field("@version", &self.version)?;
        st.serialize_field("@type", &self.server_type)?;
        st.serialize_field("@serverVersion", &self.server_version)?;
        st.serialize_field("@openSubsonic", &self.open_subsonic)?;
        if let Some(err) = &self.error {
            st.serialize_field("error", err)?;
        }
        if let Some(p) = &self.payload {
            p.serialize_into(&mut st)?;
        }
        st.end()
    }
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
pub struct Users {
    #[serde(rename = "user", default)]
    pub user: Vec<SubsonicUser>,
}

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

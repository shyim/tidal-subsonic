use crate::app::base_url_from_headers;
use crate::auth_mw::{ApiError, ApiResult, Authed};
use crate::mapping;
use crate::routes::metadata_cache::{MetadataCache, TTL_CATALOG};
use crate::subsonic::*;
use axum::http::HeaderMap;

/// A search-window request: query plus the paging attributes clients vary, used
/// both to build the cached result and to key it.
struct SearchWindow {
    query: String,
    artist_count: u32,
    artist_offset: u32,
    album_count: u32,
    album_offset: u32,
    song_count: u32,
    song_offset: u32,
}

impl SearchWindow {
    fn from(authed: &Authed) -> Result<Self, ApiError> {
        let query = match &authed.params.query {
            Some(q) if !q.is_empty() => q.clone(),
            _ => return Err(ApiError::BadRequest(10, "Missing query".to_string())),
        };
        Ok(SearchWindow {
            query,
            artist_count: authed.params.artist_count.unwrap_or(20),
            artist_offset: authed.params.artist_offset.unwrap_or(0),
            album_count: authed.params.album_count.unwrap_or(20),
            album_offset: authed.params.album_offset.unwrap_or(0),
            song_count: authed.params.song_count.unwrap_or(20),
            song_offset: authed.params.song_offset.unwrap_or(0),
        })
    }

    /// Param portion of the cache key. `base_url` is included because song URLs
    /// bake it in, so results from one host must not be served to another.
    fn cache_params(&self, base_url: &str) -> String {
        format!(
            "q={}&ac={}&ao={}&alc={}&alo={}&sc={}&so={}&base={}",
            self.query,
            self.artist_count,
            self.artist_offset,
            self.album_count,
            self.album_offset,
            self.song_count,
            self.song_offset,
            base_url,
        )
    }
}

/// Run the search and map it into the (artist, album, song) triple shared by
/// searchResult2 and searchResult3, which have identical field shapes.
async fn build_search(
    client: &crate::tidal::SharedTidalClient,
    w: &SearchWindow,
    base_url: &str,
) -> Result<
    (
        Vec<SubsonicArtist>,
        Vec<SubsonicAlbum>,
        Vec<SubsonicChild>,
    ),
    String,
> {
    let results = client.search(&w.query, 50).await?;
    let artists: Vec<SubsonicArtist> = results.artists[w.artist_offset as usize..]
        .iter()
        .take(w.artist_count as usize)
        .map(mapping::search_artist_to_subsonic)
        .collect();
    let albums: Vec<SubsonicAlbum> = results.albums[w.album_offset as usize..]
        .iter()
        .take(w.album_count as usize)
        .map(mapping::album_to_subsonic)
        .collect();
    let songs: Vec<SubsonicChild> = results.tracks[w.song_offset as usize..]
        .iter()
        .take(w.song_count as usize)
        .map(|t| mapping::track_to_child(t, base_url))
        .collect();
    Ok((artists, albums, songs))
}

pub(crate) async fn handle_search2(authed: Authed, headers: HeaderMap) -> ApiResult {
    let w = SearchWindow::from(&authed)?;
    let client = authed.tidal().await?;
    let base_url = base_url_from_headers(&headers);
    let key = MetadataCache::key(authed.user.id, "search2", &w.cache_params(&base_url));
    let (artists, albums, songs) = authed
        .state
        .metadata_cache
        .get_or_build(&key, TTL_CATALOG, || build_search(&client, &w, &base_url))
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::SearchResult2(SearchResult2 {
        artist: Some(artists),
        album: Some(albums),
        song: Some(songs),
    })
    .into())
}

pub(crate) async fn handle_search3(authed: Authed, headers: HeaderMap) -> ApiResult {
    let w = SearchWindow::from(&authed)?;
    let client = authed.tidal().await?;
    let base_url = base_url_from_headers(&headers);
    let key = MetadataCache::key(authed.user.id, "search3", &w.cache_params(&base_url));
    let (artists, albums, songs) = authed
        .state
        .metadata_cache
        .get_or_build(&key, TTL_CATALOG, || build_search(&client, &w, &base_url))
        .await
        .map_err(ApiError::Tidal)?;
    Ok(Payload::SearchResult3(SearchResult3 {
        artist: Some(artists),
        album: Some(albums),
        song: Some(songs),
    })
    .into())
}

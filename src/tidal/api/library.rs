//! Library reads and favorites mutations: playlists, albums, artists,
//! tracks, search, favorites.

use super::super::client::{TIDAL_API_URL, TIDAL_API_V2_URL};
use super::super::types::*;
use super::super::TidalClient;
use serde::Deserialize;

impl TidalClient {
    pub async fn get_user_playlists(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalPlaylist>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/playlists", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalPlaylist>,
            total_number_of_items: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn get_playlist(&self, playlist_id: &str) -> Result<TidalPlaylist, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/playlists/{}", playlist_id), &[("countryCode", &cc)])
            .await
    }

    pub async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/playlists/{}/tracks", TIDAL_API_URL, playlist_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalTrack>,
            total_number_of_items: u32,
            #[serde(default)]
            offset: u32,
            #[serde(default)]
            limit: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset: data.offset,
            limit: data.limit,
        })
    }

    pub async fn get_favorite_tracks(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/favorites/tracks", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("order", "DATE"),
                ("orderDirection", "DESC"),
            ],
        ).await?;

        #[derive(Deserialize)]
        struct FavItem {
            item: TidalTrack,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FavResponse {
            items: Vec<FavItem>,
            total_number_of_items: u32,
            #[serde(default)]
            offset: u32,
            #[serde(default)]
            limit: u32,
        }

        let data: FavResponse = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items.into_iter().map(|f| f.item).collect(),
            total_number_of_items: data.total_number_of_items,
            offset: data.offset,
            limit: data.limit,
        })
    }

    pub async fn get_favorite_albums(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalAlbumDetail>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/favorites/albums", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("order", "DATE"),
                ("orderDirection", "DESC"),
            ],
        ).await?;

        #[derive(Deserialize)]
        struct FavItem {
            item: TidalAlbumDetail,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FavResponse {
            items: Vec<FavItem>,
            total_number_of_items: u32,
        }

        let data: FavResponse = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items.into_iter().map(|f| f.item).collect(),
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn get_favorite_artists(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalArtistDetail>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/users/{}/favorites/artists", TIDAL_API_URL, user_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("order", "DATE"),
                ("orderDirection", "DESC"),
            ],
        ).await?;

        #[derive(Deserialize)]
        struct FavItem {
            item: TidalArtistDetail,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FavResponse {
            items: Vec<FavItem>,
            total_number_of_items: u32,
        }

        let data: FavResponse = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items.into_iter().map(|f| f.item).collect(),
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    /// Add an item to the user's TIDAL favorites. `kind` is the favorites
    /// collection ("tracks" | "albums" | "artists") and `id_param` the matching
    /// id field ("trackIds" | "albumIds" | "artistIds").
    pub(super) async fn add_favorite(&self, kind: &str, id_param: &str, id: u64) -> Result<(), String> {
        self.ensure_auth().await?;
        let (user_id, cc, token) = {
            let creds = self.creds.lock().await;
            (creds.user_id, creds.country_code.clone(), creds.access_token.clone())
        };
        let user_id = user_id.ok_or("Not authenticated with TIDAL")?;
        let url = format!("{}/users/{}/favorites/{}", TIDAL_API_URL, user_id, kind);
        let id_str = id.to_string();

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[("countryCode", cc.as_str())])
            .form(&[(id_param, id_str.as_str()), ("onArtifactNotFound", "FAIL")])
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Favorite add error ({}): {}", status, body));
        }
        Ok(())
    }

    /// Remove an item from the user's TIDAL favorites.
    pub(super) async fn remove_favorite(&self, kind: &str, id: u64) -> Result<(), String> {
        self.ensure_auth().await?;
        let (user_id, cc, token) = {
            let creds = self.creds.lock().await;
            (creds.user_id, creds.country_code.clone(), creds.access_token.clone())
        };
        let user_id = user_id.ok_or("Not authenticated with TIDAL")?;
        let url = format!("{}/users/{}/favorites/{}/{}", TIDAL_API_URL, user_id, kind, id);

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[("countryCode", cc.as_str())])
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        let status = resp.status();
        if !status.is_success() && status != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Favorite remove error ({}): {}", status, body));
        }
        Ok(())
    }

    pub async fn add_favorite_track(&self, id: u64) -> Result<(), String> {
        self.add_favorite("tracks", "trackIds", id).await
    }
    pub async fn add_favorite_album(&self, id: u64) -> Result<(), String> {
        self.add_favorite("albums", "albumIds", id).await
    }
    pub async fn add_favorite_artist(&self, id: u64) -> Result<(), String> {
        self.add_favorite("artists", "artistIds", id).await
    }
    pub async fn remove_favorite_track(&self, id: u64) -> Result<(), String> {
        self.remove_favorite("tracks", id).await
    }
    pub async fn remove_favorite_album(&self, id: u64) -> Result<(), String> {
        self.remove_favorite("albums", id).await
    }
    pub async fn remove_favorite_artist(&self, id: u64) -> Result<(), String> {
        self.remove_favorite("artists", id).await
    }

    pub async fn get_album_detail(&self, album_id: u64) -> Result<TidalAlbumDetail, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/albums/{}", album_id), &[("countryCode", &cc)]).await
    }

    pub async fn get_album_tracks(
        &self,
        album_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/albums/{}/tracks", TIDAL_API_URL, album_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalTrack>,
            total_number_of_items: u32,
            #[serde(default)]
            offset: u32,
            #[serde(default)]
            limit: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset: data.offset,
            limit: data.limit,
        })
    }

    pub async fn get_artist_detail(
        &self,
        artist_id: u64,
    ) -> Result<TidalArtistDetail, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/artists/{}", artist_id), &[("countryCode", &cc)]).await
    }

    pub async fn get_artist_albums(
        &self,
        artist_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<TidalAlbumDetail>, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/artists/{}/albums", TIDAL_API_URL, artist_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalAlbumDetail>,
            total_number_of_items: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedResponse {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn get_artist_top_tracks(
        &self,
        artist_id: u64,
        offset: u32,
        limit: u32,
    ) -> Result<PaginatedTracks, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/artists/{}/tracks", TIDAL_API_URL, artist_id),
            &[
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resp {
            items: Vec<TidalTrack>,
            total_number_of_items: u32,
        }

        let data: Resp = serde_json::from_str(&body).map_err(|e| format!("Parse error: {}", e))?;
        Ok(PaginatedTracks {
            items: data.items,
            total_number_of_items: data.total_number_of_items,
            offset,
            limit,
        })
    }

    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<TidalSearchResults, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/search", TIDAL_API_V2_URL),
            &[
                ("query", query),
                ("countryCode", &cc),
                ("limit", &limit.to_string()),
                ("types", "ARTISTS,ALBUMS,TRACKS,PLAYLISTS"),
                ("includeContributors", "true"),
                ("includeUserPlaylists", "true"),
                ("locale", "en_US"),
                ("deviceType", "BROWSER"),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Sec<T> {
            items: Vec<T>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SR {
            #[serde(default)]
            artists: Option<Sec<TidalArtist>>,
            #[serde(default)]
            albums: Option<Sec<TidalAlbumDetail>>,
            #[serde(default)]
            tracks: Option<Sec<TidalTrack>>,
            #[serde(default)]
            playlists: Option<Sec<TidalPlaylist>>,
        }

        let data: SR = serde_json::from_str(&body)
            .map_err(|e| format!("Search parse error: {}", e))?;

        Ok(TidalSearchResults {
            artists: data.artists.map(|s| s.items).unwrap_or_default(),
            albums: data.albums.map(|s| s.items).unwrap_or_default(),
            tracks: data.tracks.map(|s| s.items).unwrap_or_default(),
            playlists: data.playlists.map(|s| s.items).unwrap_or_default(),
        })
    }

    pub async fn get_track(&self, track_id: u64) -> Result<TidalTrack, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/tracks/{}", track_id), &[("countryCode", &cc)]).await
    }

    pub async fn get_track_lyrics(&self, track_id: u64) -> Result<TidalLyrics, String> {
        let cc = self.country_code().await;
        self.api_get(&format!("/tracks/{}/lyrics", track_id), &[("countryCode", &cc)]).await
    }
}

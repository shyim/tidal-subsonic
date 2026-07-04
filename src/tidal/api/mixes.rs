//! TIDAL generated mixes (Daily Mix, My Mix, Discovery, …). These aren't albums
//! or user playlists — they're a separate entity served by the `pages/*` API. We
//! surface them to Subsonic as playlists (`mix-<id>`).
//!
//! Page responses are `{ rows: [ { modules: [ { type, pagedList: { items } } ] } ] }`;
//! the list of mixes lives in a `MIX_LIST` module, a mix's tracks in `TRACK_LIST`.

use super::super::client::TIDAL_API_URL;
use super::super::types::{TidalMix, TidalTrack};
use super::super::TidalClient;
use serde_json::Value;

impl TidalClient {
    /// The user's generated mixes, from `pages/my_collection_my_mixes`.
    pub async fn get_my_mixes(&self) -> Result<Vec<TidalMix>, String> {
        let cc = self.country_code().await;
        let body = self
            .authenticated_get(
                &format!("{}/pages/my_collection_my_mixes", TIDAL_API_URL),
                &[
                    ("countryCode", &cc),
                    ("deviceType", "BROWSER"),
                    ("locale", "en_US"),
                ],
            )
            .await?;
        let json: Value =
            serde_json::from_str(&body).map_err(|e| format!("mixes parse: {}", e))?;

        let mut mixes = Vec::new();
        for item in page_module_items(&json, "MIX_LIST") {
            if let Ok(mix) = serde_json::from_value::<TidalMix>(item.clone()) {
                mixes.push(mix);
            }
        }
        Ok(mixes)
    }

    /// The user's mixes, each paired with its track count. The mix list itself
    /// carries no count, so we fetch each mix's tracks — but concurrently, so the
    /// added latency is roughly one fetch, not N. `self` must be `Arc` (the
    /// registry always hands out `SharedTidalClient`) so the tasks can share it.
    pub async fn get_my_mixes_with_counts(
        self: &std::sync::Arc<Self>,
    ) -> Result<Vec<(TidalMix, u32)>, String> {
        let mixes = self.get_my_mixes().await?;
        let mut set = tokio::task::JoinSet::new();
        for (idx, mix) in mixes.iter().enumerate() {
            let client = self.clone();
            let id = mix.id.clone();
            set.spawn(async move {
                let count = client
                    .get_mix_tracks(&id)
                    .await
                    .map(|d| d.tracks.len() as u32)
                    .unwrap_or(0);
                (idx, count)
            });
        }
        let mut counts = vec![0u32; mixes.len()];
        while let Some(res) = set.join_next().await {
            if let Ok((idx, count)) = res {
                counts[idx] = count;
            }
        }
        Ok(mixes.into_iter().zip(counts).collect())
    }

    /// A mix's title + tracks, from `pages/mix?mixId=…` (falls back to the legacy
    /// `mixes/{id}/items` endpoint, which has no title, if the page has no
    /// TRACK_LIST).
    pub async fn get_mix_tracks(&self, mix_id: &str) -> Result<MixDetail, String> {
        let cc = self.country_code().await;
        if let Ok(body) = self
            .authenticated_get(
                &format!("{}/pages/mix", TIDAL_API_URL),
                &[
                    ("mixId", mix_id),
                    ("countryCode", &cc),
                    ("deviceType", "BROWSER"),
                    ("locale", "en_US"),
                ],
            )
            .await
        {
            if let Ok(json) = serde_json::from_str::<Value>(&body) {
                let tracks: Vec<TidalTrack> = page_module_items(&json, "TRACK_LIST")
                    .into_iter()
                    .filter_map(|it| serde_json::from_value::<TidalTrack>(it.clone()).ok())
                    .collect();
                if !tracks.is_empty() {
                    return Ok(MixDetail {
                        title: mix_header_title(&json),
                        tracks,
                    });
                }
            }
        }

        // Fallback: legacy items endpoint (no title available).
        let body = self
            .authenticated_get(
                &format!("{}/mixes/{}/items", TIDAL_API_URL, mix_id),
                &[("countryCode", &cc)],
            )
            .await?;
        let json: Value =
            serde_json::from_str(&body).map_err(|e| format!("mix items parse: {}", e))?;
        let items = json.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
        let tracks = items
            .iter()
            // legacy items wrap the track under `item`.
            .filter_map(|it| {
                let track = it.get("item").unwrap_or(it);
                serde_json::from_value::<TidalTrack>(track.clone()).ok()
            })
            .collect();
        Ok(MixDetail { title: None, tracks })
    }
}

/// A mix's resolved title (if available) plus its tracks.
pub struct MixDetail {
    pub title: Option<String>,
    pub tracks: Vec<TidalTrack>,
}

/// Extract the mix title from a `pages/mix` response's MIX_HEADER module.
fn mix_header_title(json: &Value) -> Option<String> {
    let rows = json.get("rows")?.as_array()?;
    for row in rows {
        let modules = row.get("modules").and_then(|m| m.as_array())?;
        for module in modules {
            if module.get("type").and_then(|t| t.as_str()) == Some("MIX_HEADER") {
                return module
                    .get("mix")
                    .and_then(|m| m.get("title"))
                    .and_then(|t| t.as_str())
                    .map(String::from);
            }
        }
    }
    None
}

/// Collect the `pagedList.items` of the first module whose `type` matches
/// `module_type`, across all rows of a page response.
fn page_module_items(json: &Value, module_type: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(rows) = json.get("rows").and_then(|r| r.as_array()) else {
        return out;
    };
    for row in rows {
        let Some(modules) = row.get("modules").and_then(|m| m.as_array()) else {
            continue;
        };
        for module in modules {
            if module.get("type").and_then(|t| t.as_str()) == Some(module_type) {
                if let Some(items) = module
                    .get("pagedList")
                    .and_then(|p| p.get("items"))
                    .and_then(|i| i.as_array())
                {
                    out.extend(items.iter().cloned());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_mix_list_items() {
        let page = json!({
            "rows": [{ "modules": [{
                "type": "MIX_LIST",
                "pagedList": { "items": [
                    { "id": "abc", "title": "My Mix 1", "mixType": "DAILY_MIX" },
                    { "id": "def", "title": "Discovery", "mixType": "DISCOVERY_MIX" }
                ]}
            }]}]
        });
        let items = page_module_items(&page, "MIX_LIST");
        assert_eq!(items.len(), 2);
        let mix: TidalMix = serde_json::from_value(items[0].clone()).unwrap();
        assert_eq!(mix.id, "abc");
        assert_eq!(mix.title.as_deref(), Some("My Mix 1"));
    }

    #[test]
    fn image_url_prefers_large() {
        let mix: TidalMix = serde_json::from_value(json!({
            "id": "x",
            "images": {
                "SMALL": { "url": "s.jpg" },
                "LARGE": { "url": "l.jpg" }
            }
        }))
        .unwrap();
        assert_eq!(mix.image_url().as_deref(), Some("l.jpg"));
    }

    #[test]
    fn missing_module_yields_empty() {
        let page = json!({ "rows": [{ "modules": [{ "type": "OTHER" }] }] });
        assert!(page_module_items(&page, "MIX_LIST").is_empty());
    }
}

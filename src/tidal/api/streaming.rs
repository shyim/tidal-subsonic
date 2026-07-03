//! Playback / streaming resolution: turns a track id into a directly-
//! streamable URL or a reconstructable DASH segment list.

use super::super::client::{DASH_SEGMENTED_ERR, QUALITY_LADDER, TIDAL_API_URL};
use super::super::dash::{extract_dash_direct_url, extract_dash_segments};
use super::super::types::StreamInfo;
use super::super::TidalClient;
use serde::Deserialize;

impl TidalClient {
    pub async fn get_stream_url(
        &self,
        track_id: u64,
        quality: &str,
    ) -> Result<StreamInfo, String> {
        let cc = self.country_code().await;
        let body = self.authenticated_get(
            &format!("{}/tracks/{}/playbackinfopostpaywall", TIDAL_API_URL, track_id),
            &[
                ("countryCode", &cc),
                ("audioquality", quality),
                ("playbackmode", "STREAM"),
                ("assetpresentation", "FULL"),
            ],
        ).await?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PlaybackInfo {
            manifest_mime_type: String,
            manifest: String,
            #[serde(default)]
            audio_quality: Option<String>,
            #[serde(default)]
            bit_depth: Option<u32>,
            #[serde(default)]
            sample_rate: Option<u32>,
            #[serde(default)]
            album_replay_gain: Option<f64>,
            #[serde(default)]
            album_peak_amplitude: Option<f64>,
            #[serde(default)]
            track_replay_gain: Option<f64>,
            #[serde(default)]
            track_peak_amplitude: Option<f64>,
        }

        let data: PlaybackInfo = serde_json::from_str(&body)
            .map_err(|e| format!("Parse error: {} - Body: {}", e, body))?;

        let manifest_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &data.manifest,
        ).map_err(|e| format!("Manifest decode error: {}", e))?;

        let manifest_str = String::from_utf8(manifest_bytes)
            .map_err(|e| format!("Invalid manifest: {}", e))?;

        let mut codec: Option<String> = None;
        let url;

        if data.manifest_mime_type.contains("vnd.tidal.bts") {
            #[derive(Deserialize)]
            struct BtsManifest {
                urls: Vec<String>,
                codecs: Option<String>,
            }

            let manifest_data: BtsManifest = serde_json::from_str(&manifest_str)
                .map_err(|e| format!("BTS parse error: {}", e))?;

            codec = manifest_data.codecs
                .map(|c| c.split('.').next().unwrap_or("").to_uppercase());

            url = manifest_data.urls.into_iter().next()
                .ok_or("No URL in BTS manifest".to_string())?;
        } else if data.manifest_mime_type.contains("dash+xml") {
            if let Some(codecs_start) = manifest_str.find("codecs=\"") {
                let start = codecs_start + 8;
                if let Some(codecs_end) = manifest_str[start..].find('\"') {
                    let raw = &manifest_str[start..start + codecs_end];
                    codec = Some(if raw.contains("flac") {
                        "FLAC".to_string()
                    } else {
                        raw.to_uppercase()
                    });
                }
            }
            // First try a single direct BaseURL (some DASH manifests have one).
            url = extract_dash_direct_url(&manifest_str).unwrap_or_default();
            if url.is_empty() {
                // Otherwise reconstruct the segment list from the
                // SegmentTemplate so the caller can concatenate segments into a
                // single playable file.
                let segments = extract_dash_segments(&manifest_str);
                if segments.is_empty() {
                    return Err(DASH_SEGMENTED_ERR.to_string());
                }
                return Ok(StreamInfo {
                    url: String::new(),
                    segments,
                    codec,
                    bit_depth: data.bit_depth,
                    sample_rate: data.sample_rate,
                    audio_quality: data.audio_quality.clone(),
                    manifest: Some(manifest_str),
                    manifest_mime_type: Some(data.manifest_mime_type),
                    album_replay_gain: data.album_replay_gain,
                    album_peak_amplitude: data.album_peak_amplitude,
                    track_replay_gain: data.track_replay_gain,
                    track_peak_amplitude: data.track_peak_amplitude,
                });
            }
        } else {
            return Err(format!("Unknown manifest format: {}", data.manifest_mime_type));
        }

        if url.is_empty() {
            return Err("No stream URL found".into());
        }

        Ok(StreamInfo {
            url,
            segments: vec![],
            codec,
            bit_depth: data.bit_depth,
            sample_rate: data.sample_rate,
            audio_quality: data.audio_quality.clone(),
            manifest: Some(manifest_str),
            manifest_mime_type: Some(data.manifest_mime_type),
            album_replay_gain: data.album_replay_gain,
            album_peak_amplitude: data.album_peak_amplitude,
            track_replay_gain: data.track_replay_gain,
            track_peak_amplitude: data.track_peak_amplitude,
        })
    }

    /// Resolve a directly-streamable URL for a track, capped at `max_quality`.
    /// Walks the quality ladder downward from the requested ceiling until Tidal
    /// returns a single downloadable file (BTS or single-BaseURL DASH), so that
    /// HI-RES tracks — which come back as segmented DASH — still play through
    /// the proxy at the best format that can be served as one file.
    pub async fn get_streamable_url(
        &self,
        track_id: u64,
        max_quality: &str,
    ) -> Result<StreamInfo, String> {
        // Start at the requested ceiling, then step down.
        let start = QUALITY_LADDER
            .iter()
            .position(|q| *q == max_quality)
            .unwrap_or(0);

        let mut last_err = String::from("No stream URL found");
        for quality in &QUALITY_LADDER[start..] {
            match self.get_stream_url(track_id, quality).await {
                Ok(info) => return Ok(info),
                Err(e) if e == DASH_SEGMENTED_ERR => {
                    tracing::debug!(
                        "Track {} segmented DASH at quality {}, trying lower quality",
                        track_id,
                        quality
                    );
                    last_err = "All qualities returned segmented DASH streams".to_string();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}

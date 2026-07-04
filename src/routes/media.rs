use crate::auth_mw::{render_current, xml_error, Authed};
use crate::item_id::ItemId;
use crate::mapping;
use crate::routes::transcode;
use crate::tidal::StreamInfo;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    http::HeaderMap,
};
use reqwest::Client as ReqwestClient;

// ------ Cover Art / Image proxy ------

pub(crate) async fn handle_get_cover_art(authed: Authed) -> Response {
    let state = &authed.state;
    let cover_id = match &authed.params.id {
        Some(id) => id.clone(),
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Missing cover id").into_response();
        }
    };

    let size = authed.params.size.unwrap_or(640);
    let candidates = mapping::cover_art_urls(&cover_id, size);
    if candidates.is_empty() {
        return (StatusCode::NOT_FOUND, "Invalid cover ID").into_response();
    }

    // TIDAL serves different size sets per image kind (album vs artist), so try
    // the size-ranked candidates until one exists on the CDN.
    for image_url in &candidates {
        // SSRF guard: only ever fetch from TIDAL's image CDN, never a host a
        // crafted cover id might have smuggled in.
        if !is_allowed_cover_host(image_url) {
            tracing::warn!("Blocked non-TIDAL cover art host: {}", image_url);
            continue;
        }
        match state.http_client.get(image_url).send().await {
            Ok(response) if response.status().is_success() => {
                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/jpeg")
                    .to_string();
                if let Ok(bytes) = response.bytes().await {
                    return (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, content_type)],
                        bytes.to_vec(),
                    )
                        .into_response();
                }
            }
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!("Cover art fetch error: {} for {}", e, image_url);
            }
        }
    }

    tracing::warn!("Cover art not found for id {}", cover_id);
    (StatusCode::NOT_FOUND, "Cover image not found").into_response()
}

// ------ Streaming ------

/// `download` fetches a track as a file (used by clients like the YuMusic
/// Garmin app for offline sync). For this proxy it's identical to `stream`: we
/// don't transcode, and TIDAL only ever delivers fragmented MP4, so the
/// downloaded bytes are the same seekable file `stream` produces.
pub(crate) async fn handle_download(authed: Authed, headers: HeaderMap) -> Response {
    handle_stream(authed, headers).await
}

pub(crate) async fn handle_stream(authed: Authed, headers: HeaderMap) -> Response {
    let state = &authed.state;
    let params = &authed.params;
    let track_id_str = match &params.id {
        Some(id) => id.clone(),
        None => {
            return render_current(&xml_error(10, "Missing id parameter"));
        }
    };

    let track_id: u64 = match track_id_str.parse::<ItemId>() {
        Ok(ItemId::Track(id)) => id,
        _ => {
            return render_current(&xml_error(0, "Invalid track id"));
        }
    };

    // Determine the quality ceiling.
    //
    // TIDAL delivers LOW/HIGH as AAC-in-MP4 and LOSSLESS/HI_RES as FLAC-in-MP4.
    // We don't transcode, so we map Subsonic's transcode request onto a TIDAL
    // quality the client can already play:
    //   - format=raw (or the server's configured format)  -> server max quality
    //   - any other explicit format (mp3/aac/opus/...)     -> AAC (HIGH), since a
    //     client asking to transcode is signalling it can't take the raw codec
    //     (e.g. FLAC); AAC is the best-quality container it will accept.
    //   - maxBitRate caps quality regardless.
    let max_bit_rate = params.max_bit_rate.unwrap_or(0);
    let requested_format = params.format.as_deref().map(|s| s.to_ascii_lowercase());
    let wants_transcode = matches!(
        requested_format.as_deref(),
        Some(fmt) if fmt != "raw" && !fmt.is_empty()
    );
    // Clients that require actual MP3 (e.g. the Garmin YuMusic app, which
    // declares Media.ENCODING_MP3) get a real MP3 transcode rather than the
    // AAC-in-MP4 the proxy normally serves.
    let wants_mp3 = requested_format.as_deref() == Some("mp3");

    let ceiling = if wants_mp3 {
        // MP3 transcode decodes with symphonia, which handles AAC-LC (HIGH,
        // mp4a.40.2) but NOT HE-AAC (LOW, mp4a.40.5) — always source HIGH.
        "HIGH"
    } else if wants_transcode {
        // Client can't take the raw codec: give it AAC, bitrate-capped.
        if max_bit_rate != 0 && max_bit_rate < 128 {
            "LOW"
        } else {
            "HIGH"
        }
    } else if max_bit_rate == 0 || max_bit_rate >= 320 {
        state.max_quality.as_str()
    } else if max_bit_rate >= 128 {
        "HIGH"
    } else {
        "LOW"
    };

    let stream_info = {
        let client = &state.tidal;
        match client.get_streamable_url(track_id, ceiling).await {
            Ok(info) => info,
            Err(e) => {
                return render_current(&xml_error(0, &format!("Stream URL error: {}", e)));
            }
        }
    };

    tracing::info!(
        "Streaming track {} (quality: {:?}, codec: {:?}, segments: {})",
        track_id,
        stream_info.audio_quality,
        stream_info.codec,
        stream_info.segments.len(),
    );

    // SSRF guard: refuse to proxy any stream/segment URL that points at an
    // internal target. These come from TIDAL's signed manifest, but validate
    // defensively before we make the server fetch them.
    let all_ok = stream_info.segments.iter().all(|u| is_safe_stream_host(u))
        && (stream_info.url.is_empty() || is_safe_stream_host(&stream_info.url));
    if !all_ok {
        tracing::warn!("Blocked unsafe stream host for track {}", track_id);
        return render_current(&xml_error(0, "Refusing to proxy an unsafe stream URL"));
    }

    // Assemble (and, for MP3 clients, transcode) the whole track once, cache it
    // on disk, and serve the cached file with range support. A cache hit skips
    // both the CDN re-fetch and the transcode; range seeks read the local file.
    let bitrate = if max_bit_rate == 0 { 320 } else { max_bit_rate };
    let (content_type, cache_key) = if wants_mp3 {
        ("audio/mpeg", format!("{track_id}.mp3.{bitrate}"))
    } else {
        // Non-mp3: the assembled fMP4. Key by the sourced quality so HIGH and
        // LOSSLESS don't collide.
        ("audio/mp4", format!("{track_id}.m4a.{ceiling}"))
    };

    let http = state.http_client.clone();
    let info = stream_info;
    let path = state
        .media_cache
        .get_or_build(&cache_key, move || async move {
            let raw = fetch_full_track(&http, &info).await?;
            if wants_mp3 {
                transcode::aac_mp4_to_mp3(raw, bitrate).await
            } else {
                Ok(raw)
            }
        })
        .await;

    let path = match path {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("media build failed for track {}: {}", track_id, e);
            return render_current(&xml_error(0, &format!("Media error: {}", e)));
        }
    };

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    serve_file_with_range(&path, content_type, range.as_deref()).await
}

/// Serve an on-disk file with `Accept-Ranges` and single-range `206` support,
/// reading only the requested slice from disk.
async fn serve_file_with_range(path: &std::path::Path, content_type: &str, range: Option<&str>) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => return (StatusCode::NOT_FOUND, format!("cache read: {}", e)).into_response(),
    };
    let total = match file.metadata().await {
        Ok(m) => m.len(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("stat: {}", e)).into_response(),
    };

    let parsed = range.and_then(|r| parse_byte_range(r, total));

    let mut out_headers = axum::http::HeaderMap::new();
    out_headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    out_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    out_headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());

    match parsed {
        Some((start, end)) => {
            let len = end - start + 1;
            if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "seek").into_response();
            }
            let mut buf = vec![0u8; len as usize];
            if file.read_exact(&mut buf).await.is_err() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "read").into_response();
            }
            out_headers.insert(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", start, end, total).parse().unwrap(),
            );
            out_headers.insert(header::CONTENT_LENGTH, len.to_string().parse().unwrap());
            (StatusCode::PARTIAL_CONTENT, out_headers, buf).into_response()
        }
        None => {
            let mut buf = Vec::with_capacity(total as usize);
            if file.read_to_end(&mut buf).await.is_err() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "read").into_response();
            }
            out_headers.insert(header::CONTENT_LENGTH, total.to_string().parse().unwrap());
            (StatusCode::OK, out_headers, buf).into_response()
        }
    }
}

/// Fetch a whole track into memory, whether it comes as DASH segments or a
/// single URL. Used by the MP3 transcode path (which needs the full buffer).
async fn fetch_full_track(client: &ReqwestClient, info: &StreamInfo) -> Result<Vec<u8>, String> {
    if !info.segments.is_empty() {
        return fetch_all_segments(client, &info.segments).await;
    }
    let resp = client
        .get(&info.url)
        .send()
        .await
        .map_err(|e| format!("stream fetch error: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("upstream HTTP {}", resp.status()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("stream body error: {}", e))
}

/// Fetch every DASH segment in order and concatenate into one fMP4 buffer.
/// Single tracks are ~9-40 MB, so buffering is acceptable.
async fn fetch_all_segments(client: &ReqwestClient, segments: &[String]) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    for (idx, seg_url) in segments.iter().enumerate() {
        let resp = client
            .get(seg_url)
            .send()
            .await
            .map_err(|e| format!("segment {} fetch error: {}", idx, e))?;
        if !resp.status().is_success() {
            return Err(format!("segment {} HTTP {}", idx, resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("segment {} body error: {}", idx, e))?;
        buf.extend_from_slice(&bytes);
    }
    Ok(buf)
}

/// Parse a `Range: bytes=start-end` header into inclusive (start, end) byte
/// offsets, clamped to the content length. Returns None for absent/unsatisfiable
/// or open-ended-from-zero ranges (treated as a full 200 response).
fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    // Only handle the first range in a possibly comma-separated list.
    let first = spec.split(',').next()?.trim();
    let (start_s, end_s) = first.split_once('-')?;

    let (start, end) = if start_s.is_empty() {
        // suffix range: last N bytes
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return None;
        }
        (total.saturating_sub(n), total - 1)
    } else {
        let start: u64 = start_s.parse().ok()?;
        let end: u64 = if end_s.is_empty() {
            total - 1
        } else {
            end_s.parse().ok()?
        };
        (start, end.min(total - 1))
    };

    if start > end || start >= total {
        return None;
    }
    // A plain "bytes=0-" over the whole file is best served as a normal 200.
    if start == 0 && end == total - 1 {
        return None;
    }
    Some((start, end))
}

/// Cover-art IDs are user-supplied (a base64 blob that may decode to a full
/// URL), so getCoverArt could be steered at an arbitrary host — an SSRF vector.
/// Restrict cover fetches to TIDAL's image CDN.
fn is_allowed_cover_host(url: &str) -> bool {
    match reqwest::Url::parse(url) {
        Ok(u) => matches!(u.host_str(), Some(h) if h == "resources.tidal.com" || h.ends_with(".tidal.com")),
        Err(_) => false,
    }
}

/// Stream URLs come from TIDAL's own signed playbackinfo manifest, not from
/// user input, so a strict allowlist would break legitimate regional CDNs.
/// Instead just refuse obviously-internal targets (loopback, private, and
/// link-local ranges — the classic SSRF pivots).
fn is_safe_stream_host(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    match u.host() {
        Some(url::Host::Domain(h)) => {
            let h = h.to_ascii_lowercase();
            h != "localhost" && !h.ends_with(".localhost") && !h.ends_with(".internal")
        }
        Some(url::Host::Ipv4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
        }
        Some(url::Host::Ipv6(ip)) => !(ip.is_loopback() || ip.is_unspecified()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_host_allowlist() {
        assert!(is_allowed_cover_host("https://resources.tidal.com/images/a/640x640.jpg"));
        assert!(is_allowed_cover_host("https://foo.tidal.com/x.jpg"));
        assert!(!is_allowed_cover_host("http://169.254.169.254/latest/meta-data/"));
        assert!(!is_allowed_cover_host("https://evil.example.com/x.jpg"));
        assert!(!is_allowed_cover_host("https://resources.tidal.com.evil.com/x"));
    }

    #[test]
    fn stream_host_blocks_internal() {
        assert!(is_safe_stream_host("https://sp-ad-fa.audio.tidal.com/mediatracks/x/0.mp4"));
        assert!(is_safe_stream_host("https://cdn.cloudfront.net/x.mp4")); // regional CDNs allowed
        assert!(!is_safe_stream_host("http://localhost/x"));
        assert!(!is_safe_stream_host("http://127.0.0.1/x"));
        assert!(!is_safe_stream_host("http://169.254.169.254/latest/meta-data/"));
        assert!(!is_safe_stream_host("http://10.0.0.5/x"));
        assert!(!is_safe_stream_host("http://192.168.1.1/x"));
        assert!(!is_safe_stream_host("file:///etc/passwd"));
    }

    #[test]
    fn range_full_from_zero_is_treated_as_200() {
        // "bytes=0-" over the whole file → None so we serve a plain 200.
        assert_eq!(parse_byte_range("bytes=0-", 1000), None);
    }

    #[test]
    fn range_explicit_window_is_inclusive() {
        assert_eq!(parse_byte_range("bytes=100-199", 1000), Some((100, 199)));
    }

    #[test]
    fn range_open_ended_and_clamped() {
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_byte_range("bytes=900-5000", 1000), Some((900, 999)));
    }

    #[test]
    fn range_suffix_returns_last_n_bytes() {
        assert_eq!(parse_byte_range("bytes=-100", 1000), Some((900, 999)));
    }

    #[test]
    fn range_invalid_or_unsatisfiable_is_none() {
        assert_eq!(parse_byte_range("bytes=2000-3000", 1000), None);
        assert_eq!(parse_byte_range("bytes=abc", 1000), None);
        assert_eq!(parse_byte_range("kbytes=0-10", 1000), None);
        assert_eq!(parse_byte_range("bytes=0-10", 0), None);
    }
}

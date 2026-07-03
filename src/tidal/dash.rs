//! DASH/MPD manifest parsing for TIDAL streams.

pub(crate) fn extract_dash_direct_url(manifest: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(manifest).ok()?;

    let mut current_url: Option<String> = None;

    for node in doc.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "BaseURL" => {
                if let Some(text) = node.text() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        if let Ok(parsed) = reqwest::Url::parse(&text) {
                            current_url = Some(parsed.to_string());
                        } else if let Some(ref base) = current_url {
                            if let Ok(parsed) = reqwest::Url::parse(base) {
                                if let Ok(joined) = parsed.join(&text) {
                                    current_url = Some(joined.to_string());
                                }
                            }
                        }
                    }
                }
            }
            "Representation" => {
                // Check if this representation has SegmentTemplate or SegmentList - skip those
                let has_segments = node
                    .children()
                    .any(|c| {
                        c.is_element()
                            && (c.tag_name().name() == "SegmentTemplate"
                                || c.tag_name().name() == "SegmentList")
                    });
                if has_segments {
                    continue;
                }

                // Look for BaseURL in this representation
                let url = node
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "BaseURL")
                    .and_then(|c| c.text())
                    .map(|t| t.trim().to_string())
                    .or_else(|| current_url.clone());

                if let Some(u) = url {
                    if is_direct_http_media_url(&u) {
                        return Some(u);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn is_direct_http_media_url(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let path = url.path().to_ascii_lowercase();
    !path.ends_with('/') && !path.ends_with(".mpd")
}

/// Reconstruct the ordered list of segment URLs from a DASH `SegmentTemplate`
/// (as used by TIDAL). The returned list is the init segment followed by every
/// media segment; concatenating their bytes yields a single playable MP4/M4A.
///
/// Handles the `$Number$` placeholder with `startNumber` and a `SegmentTimeline`
/// that expresses segment counts via `<S d=".." r="N"/>` (repeat) entries.
pub(crate) fn extract_dash_segments(manifest: &str) -> Vec<String> {
    let Ok(doc) = roxmltree::Document::parse(manifest) else {
        return vec![];
    };

    for tmpl in doc
        .descendants()
        .filter(|n| n.has_tag_name("SegmentTemplate"))
    {
        let Some(media) = tmpl.attribute("media") else {
            continue;
        };
        let init = tmpl.attribute("initialization");
        let start_number: u64 = tmpl
            .attribute("startNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        // Count media segments from the SegmentTimeline: each <S> contributes
        // 1 + r segments (r defaults to 0).
        let mut count: u64 = 0;
        if let Some(timeline) = tmpl
            .children()
            .find(|c| c.has_tag_name("SegmentTimeline"))
        {
            for s in timeline.children().filter(|c| c.has_tag_name("S")) {
                let r: i64 = s.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(0);
                // r can be negative (unknown/until-end); treat as a single segment.
                count += 1 + r.max(0) as u64;
            }
        }

        if count == 0 || !media.contains("$Number$") {
            continue;
        }

        let mut urls = Vec::with_capacity(count as usize + 1);
        if let Some(init) = init {
            urls.push(init.to_string());
        }
        for i in 0..count {
            let n = start_number + i;
            urls.push(media.replace("$Number$", &n.to_string()));
        }
        return urls;
    }

    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SegmentTemplate manifest shaped like TIDAL's: init + $Number$ media,
    /// startNumber=1, and a timeline of `<S d=".." r="79"/><S d=".."/>` → 81
    /// media segments (80 from the first entry's 1+79, plus 1).
    const SEGMENTED: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011">
  <Period><AdaptationSet><Representation id="0" codecs="mp4a.40.2">
    <SegmentTemplate timescale="44100"
      initialization="https://cdn.tidal.com/x/init.mp4"
      media="https://cdn.tidal.com/x/$Number$.mp4" startNumber="1">
      <SegmentTimeline><S d="176128" r="79"/><S d="42562"/></SegmentTimeline>
    </SegmentTemplate>
  </Representation></AdaptationSet></Period>
</MPD>"#;

    /// A manifest whose Representation has a single direct BaseURL (no segments).
    const DIRECT: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011">
  <Period><AdaptationSet><Representation id="0" codecs="flac">
    <BaseURL>https://cdn.tidal.com/x/track.flac</BaseURL>
  </Representation></AdaptationSet></Period>
</MPD>"#;

    #[test]
    fn segments_reconstructs_init_plus_numbered_media() {
        let urls = extract_dash_segments(SEGMENTED);
        // 1 init + 81 media = 82 URLs.
        assert_eq!(urls.len(), 82);
        assert_eq!(urls[0], "https://cdn.tidal.com/x/init.mp4");
        assert_eq!(urls[1], "https://cdn.tidal.com/x/1.mp4");
        assert_eq!(urls[81], "https://cdn.tidal.com/x/81.mp4");
    }

    #[test]
    fn segments_empty_for_direct_manifest() {
        // No SegmentTemplate → nothing to reconstruct.
        assert!(extract_dash_segments(DIRECT).is_empty());
    }

    #[test]
    fn segments_empty_for_garbage() {
        assert!(extract_dash_segments("not xml at all").is_empty());
    }

    #[test]
    fn direct_url_extracted_from_single_baseurl() {
        assert_eq!(
            extract_dash_direct_url(DIRECT).as_deref(),
            Some("https://cdn.tidal.com/x/track.flac")
        );
    }

    #[test]
    fn direct_url_none_for_segmented() {
        // A SegmentTemplate-only representation has no single downloadable URL.
        assert_eq!(extract_dash_direct_url(SEGMENTED), None);
    }

    #[test]
    fn is_direct_media_url_rules() {
        assert!(is_direct_http_media_url("https://cdn.tidal.com/x/track.flac"));
        assert!(!is_direct_http_media_url("https://cdn.tidal.com/x/")); // ends with /
        assert!(!is_direct_http_media_url("https://cdn.tidal.com/x.mpd")); // manifest, not media
        assert!(!is_direct_http_media_url("ftp://cdn.tidal.com/x.flac")); // wrong scheme
    }
}

use crate::auth_mw::{ApiError, ApiResult, Authed};
use crate::item_id::ItemId;
use crate::routes::metadata_cache::{MetadataCache, TTL_CATALOG};
use crate::subsonic::*;
use crate::tidal::types::TidalLyrics;

/// Fetch (and cache) a track's lyrics from TIDAL. Returns `None` when the track
/// has no lyrics (TIDAL answers such tracks with an error, which we swallow so
/// the Subsonic response is an empty-but-successful lyrics result).
async fn fetch_lyrics(authed: &Authed, track_id: u64) -> Option<TidalLyrics> {
    let client = authed.tidal().await.ok()?;
    let key = MetadataCache::key(authed.user.id, "getLyrics", &format!("id={track_id}"));
    authed
        .state
        .metadata_cache
        .get_or_build(&key, TTL_CATALOG, || async {
            client.get_track_lyrics(track_id).await
        })
        .await
        .ok()
}

/// Parse an LRC-formatted subtitle block into synced lines. Each source line
/// looks like `[mm:ss.xx] text`; the timestamp becomes the line's `start`
/// offset in milliseconds. Lines without a leading timestamp are kept untimed.
fn parse_lrc(subtitles: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in subtitles.lines() {
        let raw = raw.trim_end();
        if raw.is_empty() {
            continue;
        }
        if let Some((start, text)) = parse_lrc_line(raw) {
            lines.push(LyricLine {
                start: Some(start),
                value: text.to_string(),
            });
        } else {
            lines.push(LyricLine {
                start: None,
                value: raw.to_string(),
            });
        }
    }
    lines
}

/// Parse a single `[mm:ss.xx]text` line into `(start_ms, text)`. Returns `None`
/// when the line does not start with a well-formed `[..]` timestamp tag.
fn parse_lrc_line(line: &str) -> Option<(i64, &str)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    let (tag, text) = (&rest[..close], rest[close + 1..].trim_start());
    let (mins, secs) = tag.split_once(':')?;
    let mins: i64 = mins.trim().parse().ok()?;
    let secs: f64 = secs.trim().parse().ok()?;
    let ms = mins * 60_000 + (secs * 1000.0).round() as i64;
    Some((ms, text))
}

/// Split plain-text lyrics into one untimed `<line>` per source line.
fn plain_lines(lyrics: &str) -> Vec<LyricLine> {
    lyrics
        .lines()
        .map(|l| LyricLine {
            start: None,
            value: l.trim_end().to_string(),
        })
        .collect()
}

/// Build a single `structuredLyrics` block from a TIDAL lyrics record,
/// preferring the LRC subtitles (synced) over the plain body. Returns `None`
/// when neither field carries usable text.
fn to_structured(lyrics: &TidalLyrics) -> Option<StructuredLyrics> {
    let (synced, lines) = match lyrics.subtitles.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(subs) => (true, parse_lrc(subs)),
        None => {
            let body = lyrics.lyrics.as_deref().filter(|s| !s.trim().is_empty())?;
            (false, plain_lines(body))
        }
    };
    if lines.is_empty() {
        return None;
    }
    Some(StructuredLyrics {
        lang: "xxx".to_string(),
        synced,
        display_artist: None,
        display_title: None,
        offset: None,
        line: lines,
    })
}

/// Flatten a TIDAL lyrics record to a single plain-text string for the classic
/// `getLyrics` element body, preferring the plain body over stripped subtitles.
fn plain_text(lyrics: &TidalLyrics) -> String {
    if let Some(body) = lyrics.lyrics.as_deref().filter(|s| !s.trim().is_empty()) {
        return body.to_string();
    }
    match lyrics.subtitles.as_deref() {
        Some(subs) => parse_lrc(subs)
            .into_iter()
            .map(|l| l.value)
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    }
}

/// Classic Subsonic `getLyrics`: takes `artist` + `title`, or an `id` (a TIDAL
/// track id, as used by clients that pass through the song id). Returns a
/// `<lyrics artist=".." title="..">TEXT</lyrics>` element. When no track can be
/// resolved (or it has no lyrics), an empty `<lyrics/>` is returned.
pub(crate) async fn handle_get_lyrics(authed: Authed) -> ApiResult {
    // Resolve a track id from the `id` param when present (some clients send the
    // song id here). `artist`/`title` are echoed back but not used for lookup —
    // TIDAL has no artist+title lyrics search, so text is only returned when an
    // id resolves to a track.
    let track_id = authed
        .params
        .id
        .as_deref()
        .and_then(|s| match s.parse::<ItemId>() {
            Ok(ItemId::Track(id)) => Some(id),
            _ => None,
        });

    let artist = authed.params.artist.clone();
    let title = authed.params.title.clone();

    let value = match track_id {
        Some(id) => fetch_lyrics(&authed, id)
            .await
            .map(|l| plain_text(&l))
            .unwrap_or_default(),
        None => String::new(),
    };

    Ok(Payload::Lyrics(Lyrics {
        artist,
        title,
        value,
    })
    .into())
}

/// OpenSubsonic `getLyricsBySongId`: takes `id` (a TIDAL track id) and returns a
/// `<lyricsList>` with a single `<structuredLyrics>` block (synced when TIDAL
/// provides LRC subtitles, plain otherwise). An unknown/lyric-less track yields
/// an empty `<lyricsList/>`.
pub(crate) async fn handle_get_lyrics_by_song_id(authed: Authed) -> ApiResult {
    let track_id_str = authed
        .params
        .id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest(10, "Missing song id".to_string()))?;
    let track_id = match track_id_str.parse::<ItemId>() {
        Ok(ItemId::Track(id)) => id,
        _ => return Err(ApiError::BadRequest(0, "Invalid song id".to_string())),
    };

    let structured = fetch_lyrics(&authed, track_id)
        .await
        .as_ref()
        .and_then(to_structured);

    Ok(Payload::LyricsList(LyricsList {
        structured_lyrics: structured.into_iter().collect(),
    })
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lrc_synced_lines() {
        let subs = "[00:12.34]Hello world\n[01:05.00]Second line";
        let lines = parse_lrc(subs);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start, Some(12_340));
        assert_eq!(lines[0].value, "Hello world");
        assert_eq!(lines[1].start, Some(65_000));
        assert_eq!(lines[1].value, "Second line");
    }

    #[test]
    fn parse_lrc_untimed_line_kept() {
        let lines = parse_lrc("no timestamp here");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start, None);
        assert_eq!(lines[0].value, "no timestamp here");
    }

    #[test]
    fn structured_prefers_synced_subtitles() {
        let lyrics = TidalLyrics {
            lyrics: Some("plain body".to_string()),
            subtitles: Some("[00:01.00]synced".to_string()),
        };
        let s = to_structured(&lyrics).unwrap();
        assert!(s.synced);
        assert_eq!(s.line.len(), 1);
        assert_eq!(s.line[0].start, Some(1_000));
        assert_eq!(s.line[0].value, "synced");
    }

    #[test]
    fn structured_falls_back_to_plain() {
        let lyrics = TidalLyrics {
            lyrics: Some("line one\nline two".to_string()),
            subtitles: None,
        };
        let s = to_structured(&lyrics).unwrap();
        assert!(!s.synced);
        assert_eq!(s.line.len(), 2);
        assert_eq!(s.line[0].start, None);
    }

    #[test]
    fn structured_none_when_empty() {
        let lyrics = TidalLyrics {
            lyrics: None,
            subtitles: None,
        };
        assert!(to_structured(&lyrics).is_none());
    }

    #[test]
    fn plain_text_prefers_body() {
        let lyrics = TidalLyrics {
            lyrics: Some("the body".to_string()),
            subtitles: Some("[00:01.00]ignored".to_string()),
        };
        assert_eq!(plain_text(&lyrics), "the body");
    }

    #[test]
    fn plain_text_strips_lrc_timestamps() {
        let lyrics = TidalLyrics {
            lyrics: None,
            subtitles: Some("[00:01.00]a\n[00:02.00]b".to_string()),
        };
        assert_eq!(plain_text(&lyrics), "a\nb");
    }
}

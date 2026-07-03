use crate::subsonic::SubsonicResponse;
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;
use serde_json::{Map, Value};

/// Response format requested by the client via the `f` query parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    Xml,
    Json,
    Jsonp,
}

impl ResponseFormat {
    pub fn from_param(f: Option<&str>) -> Self {
        match f.map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("json") => ResponseFormat::Json,
            Some("jsonp") => ResponseFormat::Jsonp,
            _ => ResponseFormat::Xml,
        }
    }
}

/// Serialize a Subsonic response into the format the client asked for and turn
/// it into an axum `Response`. `callback` is only used for the JSONP format.
pub fn render(resp: &SubsonicResponse, format: ResponseFormat, callback: Option<&str>) -> Response {
    match format {
        ResponseFormat::Xml => (
            StatusCode::OK,
            [("content-type", "application/xml; charset=utf-8")],
            serialize_to_xml(resp),
        )
            .into_response(),
        ResponseFormat::Json => (
            StatusCode::OK,
            [("content-type", "application/json; charset=utf-8")],
            serialize_to_json(resp),
        )
            .into_response(),
        ResponseFormat::Jsonp => {
            let cb = callback.filter(|c| is_valid_callback(c)).unwrap_or("callback");
            let body = format!("{}({});", cb, serialize_to_json(resp));
            (
                StatusCode::OK,
                [("content-type", "application/javascript; charset=utf-8")],
                body,
            )
                .into_response()
        }
    }
}

fn is_valid_callback(cb: &str) -> bool {
    !cb.is_empty()
        && cb
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.')
}

pub fn serialize_to_xml(resp: &SubsonicResponse) -> String {
    use serde::Serialize;
    let mut buffer = String::new();
    let ser = quick_xml::se::Serializer::new(&mut buffer);
    match resp.serialize(ser) {
        Ok(_) => {
            let body = buffer
                .trim_start_matches(r#"<?xml version="1.0" encoding="UTF-8"?>"#)
                .trim_start_matches(r#"<?xml version="1.0" encoding="utf-8"?>"#)
                .trim_start();
            let mut result = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
            result.push('\n');
            result.push_str(body);
            result
        }
        Err(e) => {
            tracing::error!("XML serialization error: {}", e);
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<subsonic-response xmlns="http://subsonic.org/restapi" status="failed" version="1.16.1" type="tidal-subsonic" serverVersion="0.1.0" openSubsonic="true"><error code="0" message="Internal serialization error: {}"/></subsonic-response>"#,
                e
            )
        }
    }
}

/// Serialize to the Subsonic JSON shape: `{"subsonic-response": {...}}` with the
/// XML-oriented `@`/`$` key prefixes stripped so keys are plain names.
pub fn serialize_to_json(resp: &SubsonicResponse) -> String {
    let value = match serde_json::to_value(resp) {
        Ok(v) => strip_xml_prefixes(v),
        Err(e) => {
            tracing::error!("JSON serialization error: {}", e);
            let mut err = Map::new();
            err.insert("status".into(), Value::String("failed".into()));
            err.insert("version".into(), Value::String("1.16.1".into()));
            let mut error = Map::new();
            error.insert("code".into(), Value::from(0));
            error.insert(
                "message".into(),
                Value::String(format!("Internal serialization error: {}", e)),
            );
            err.insert("error".into(), Value::Object(error));
            Value::Object(err)
        }
    };

    let mut root = Map::new();
    root.insert("subsonic-response".into(), value);
    Value::Object(root).to_string()
}

/// Recursively remove the `@` (XML attribute) and `$value` (XML text) key
/// prefixes that quick-xml serde uses, producing idiomatic Subsonic JSON.
fn strip_xml_prefixes(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (key, val) in map {
                let clean = if let Some(stripped) = key.strip_prefix('@') {
                    stripped.to_string()
                } else if key == "$value" || key == "$text" {
                    "value".to_string()
                } else {
                    key
                };
                out.insert(clean, strip_xml_prefixes(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(strip_xml_prefixes).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subsonic::*;

    fn sample_album() -> SubsonicAlbum {
        SubsonicAlbum {
            id: "al-1".into(),
            name: "Test".into(),
            artist: Some("Artist".into()),
            artist_id: Some("ar-2".into()),
            cover_art: None,
            song_count: Some(10),
            duration: Some(2400),
            created: None,
            year: Some(2020),
            genre: None,
            starred: None,
        }
    }

    #[test]
    fn json_strips_attribute_prefixes() {
        let resp = SubsonicResponse::ok_with(Payload::AlbumList(AlbumList {
            album: vec![sample_album()],
        }));

        let json = serialize_to_json(&resp);
        let v: Value = serde_json::from_str(&json).unwrap();
        let album = &v["subsonic-response"]["albumList"]["album"][0];

        // Attribute keys must be plain, not @-prefixed.
        assert_eq!(album["id"], "al-1");
        assert_eq!(album["songCount"], 10);
        assert!(album.get("@id").is_none(), "found @-prefixed key: {json}");
        assert_eq!(v["subsonic-response"]["status"], "ok");
    }

    #[test]
    fn json_wraps_root_and_carries_error() {
        let resp = SubsonicResponse::error(40, "Wrong username or password");
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert_eq!(v["subsonic-response"]["status"], "failed");
        assert_eq!(v["subsonic-response"]["error"]["code"], 40);
        assert_eq!(
            v["subsonic-response"]["error"]["message"],
            "Wrong username or password"
        );
    }

    // ------ Round-trip: the payload enum must reproduce the exact historical
    // wire names in BOTH JSON keys and XML elements. ------

    #[test]
    fn ping_ok_has_no_payload() {
        let resp = SubsonicResponse::ok();
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        let obj = v["subsonic-response"].as_object().unwrap();
        // status + the five other fixed attrs only, no error, no payload.
        assert_eq!(obj["status"], "ok");
        assert!(obj.get("error").is_none());
        assert_eq!(obj.len(), 6, "unexpected payload keys: {obj:?}");
        assert!(xml.contains("<subsonic-response"), "xml={xml}");
        assert!(xml.contains(r#"status="ok""#), "xml={xml}");
        // No payload: the root is self-closing (attributes only, no children).
        assert!(xml.contains("/>"), "expected self-closing root, xml={xml}");
    }

    #[test]
    fn get_license_roundtrip_names() {
        let resp = SubsonicResponse::ok_with(Payload::License(License {
            valid: true,
            email: None,
            license_expires: None,
            trial_expires: None,
        }));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert!(v["subsonic-response"]["license"]["valid"] == true);
        assert!(xml.contains("<license "), "xml={xml}");
    }

    #[test]
    fn get_album_list_roundtrip_names() {
        let resp = SubsonicResponse::ok_with(Payload::AlbumList(AlbumList {
            album: vec![sample_album()],
        }));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert_eq!(v["subsonic-response"]["albumList"]["album"][0]["id"], "al-1");
        assert!(xml.contains("<albumList>"), "xml={xml}");
        assert!(xml.contains(r#"<album id="al-1""#), "xml={xml}");
    }

    #[test]
    fn get_album_roundtrip_names() {
        let resp = SubsonicResponse::ok_with(Payload::Album(AlbumWithSongs {
            id: "al-9".into(),
            name: "A".into(),
            artist: None,
            artist_id: None,
            cover_art: None,
            song_count: Some(0),
            duration: None,
            year: None,
            genre: None,
            song: vec![],
        }));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert_eq!(v["subsonic-response"]["album"]["id"], "al-9");
        assert!(xml.contains(r#"<album id="al-9""#), "xml={xml}");
    }

    #[test]
    fn search3_roundtrip_names() {
        let resp = SubsonicResponse::ok_with(Payload::SearchResult3(SearchResult3 {
            artist: Some(vec![]),
            album: Some(vec![sample_album()]),
            song: Some(vec![]),
        }));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert_eq!(
            v["subsonic-response"]["searchResult3"]["album"][0]["id"],
            "al-1"
        );
        assert!(xml.contains("<searchResult3>"), "xml={xml}");
    }

    #[test]
    fn open_subsonic_extensions_roundtrip_names() {
        let resp = SubsonicResponse::ok_with(Payload::OpenSubsonicExtensions(vec![]));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert!(v["subsonic-response"]
            .as_object()
            .unwrap()
            .contains_key("openSubsonicExtensions"));
        // Empty extension list has no child elements but the response is valid.
        assert!(xml.contains("<subsonic-response"), "xml={xml}");
    }

    #[test]
    fn get_music_directory_roundtrip_names() {
        let resp = SubsonicResponse::ok_with(Payload::Directory(Directory {
            id: "ar-5".into(),
            name: "Artist".into(),
            parent: None,
            play_count: None,
            child: vec![],
        }));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        assert_eq!(v["subsonic-response"]["directory"]["id"], "ar-5");
        assert!(xml.contains(r#"<directory id="ar-5""#), "xml={xml}");
    }

    #[test]
    fn get_starred_emits_both_starred_and_starred2() {
        let resp = SubsonicResponse::ok_with(Payload::Starred(
            Starred {
                artist: None,
                album: Some(vec![sample_album()]),
                song: None,
            },
            Starred2 {
                artist: None,
                album: Some(vec![sample_album()]),
                song: None,
            },
        ));
        let xml = serialize_to_xml(&resp);
        let v: Value = serde_json::from_str(&serialize_to_json(&resp)).unwrap();
        let obj = v["subsonic-response"].as_object().unwrap();
        assert!(obj.contains_key("starred"), "json={v}");
        assert!(obj.contains_key("starred2"), "json={v}");
        assert!(xml.contains("<starred>"), "xml={xml}");
        assert!(xml.contains("<starred2>"), "xml={xml}");
    }
}

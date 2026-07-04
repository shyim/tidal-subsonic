//! TIDAL playback event reporting ("scrobbling"). TIDAL tracks plays via a
//! batched event pipeline (SQS `SendMessageBatch` proxied through
//! `tidal.com/api/event-batch`). Reverse-engineered from the web player; see
//! TIDAL_EVENT_SYSTEM.md. We emit the minimal pair — `streaming_session_start`
//! + `streaming_session_end` — which is enough for TIDAL to register a play.

use super::super::TidalClient;
use rand::Rng;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const EVENT_BATCH_URL: &str = "https://tidal.com/api/event-batch";
/// How this proxy identifies itself in the event stream.
const APP_NAME: &str = "tidal-subsonic";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A random UUID v4 string, from `rand` (no uuid crate needed).
fn uuid_v4() -> String {
    let mut b: [u8; 16] = rand::thread_rng().gen();
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 1
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Local date/time in UTC (YYYY-MM-DD, HH:MM:SS) derived from epoch millis —
/// avoids a chrono dependency. TIDAL only uses these for coarse analytics.
fn utc_date_time(ms: u64) -> (String, String) {
    let secs = ms / 1000;
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);

    // Civil date from days-since-epoch (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    (
        format!("{:04}-{:02}-{:02}", year, month, d),
        format!("{:02}:{:02}:{:02}", h, m, s),
    )
}

impl TidalClient {
    /// Best-effort: report that `track_id` was played, so it counts toward the
    /// user's TIDAL play history. Emits a start+end streaming session pair.
    /// Errors are logged, never propagated — a failed report must not fail the
    /// Subsonic scrobble call.
    pub async fn report_play(&self, track_id: u64) {
        if let Err(e) = self.try_report_play(track_id).await {
            tracing::debug!("TIDAL play report for track {} failed: {}", track_id, e);
        }
    }

    async fn try_report_play(&self, track_id: u64) -> Result<(), String> {
        self.ensure_auth().await?;
        let access_token = self.access_token().await;
        let user_id = self.user_id().await.ok_or("no TIDAL user id")?;

        let session_id = uuid_v4();
        let ts = epoch_millis();
        let product = track_id.to_string();

        let start_payload = json!({
            "streamingSessionId": session_id,
            "timestamp": ts,
            "sessionProductId": product,
            "sessionProductType": "TRACK",
            "sessionType": "PLAYBACK",
            "startReason": "EXPLICIT",
            "networkType": "ETHERNET",
            "hardwarePlatform": "WEB",
            "browser": "",
            "browserVersion": "",
            "operatingSystem": "Linux",
            "operatingSystemVersion": "",
            "sessionTags": [],
            "isOfflineModeStart": false,
        });
        let end_payload = json!({
            "streamingSessionId": session_id,
            "timestamp": ts,
        });

        let mut form: Vec<(String, String)> = Vec::new();
        self.push_event(&mut form, 1, "streaming_session_start", ts, &access_token, user_id, start_payload);
        self.push_event(&mut form, 2, "streaming_session_end", ts, &access_token, user_id, end_payload);

        // TIDAL's edge (CloudFront) blocks requests without a browser-like
        // User-Agent and the web player's Origin, so mimic the web client.
        let resp = self
            .client
            .post(EVENT_BATCH_URL)
            .bearer_auth(&access_token)
            .header(
                "User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
            )
            .header("Origin", "https://listen.tidal.com")
            .header("Referer", "https://listen.tidal.com/")
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("network error: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("event-batch {}: {}", status, body));
        }
        Ok(())
    }

    /// Append one SQS batch entry (envelope + message attributes) for `event`.
    #[allow(clippy::too_many_arguments)]
    fn push_event(
        &self,
        form: &mut Vec<(String, String)>,
        n: usize,
        name: &str,
        ts: u64,
        access_token: &str,
        user_id: u64,
        payload: serde_json::Value,
    ) {
        // The client_id the tokens were issued for (fall back to the app default).
        let client_id = if self.client_id.is_empty() {
            "6BDSRdpK9hqEBTgU"
        } else {
            self.client_id.as_str()
        };
        let (local_date, local_time) = utc_date_time(ts);
        let envelope = json!({
            "group": "streaming_metrics",
            "name": name,
            "version": 2,
            "ts": ts,
            "uuid": uuid_v4(),
            "client": {
                "localDate": local_date,
                "localTime": local_time,
                "platform": "web",
                "system": { "browserName": "", "browserVersion": "", "osName": "Linux" },
                "timeOffset": "+00:00",
                "token": client_id,
                "version": format!("{}@{}", APP_NAME, APP_VERSION),
            },
            "user": {
                "accessToken": access_token,
                "clientId": client_id,
                "id": user_id,
            },
            "payload": payload,
        });
        let headers = json!({
            "app-name": APP_NAME,
            "app-version": APP_VERSION,
            "browser-name": "",
            "browser-version": "",
            "client-id": client_id,
            "consent-category": "NECESSARY",
            "os-name": "Linux",
            "requested-sent-timestamp": ts,
            "authorization": access_token,
        });

        let p = format!("SendMessageBatchRequestEntry.{n}");
        form.push((format!("{p}.Id"), uuid_v4()));
        form.push((format!("{p}.MessageBody"), envelope.to_string()));
        form.push((format!("{p}.MessageAttribute.1.Name"), "Name".into()));
        form.push((format!("{p}.MessageAttribute.1.Value.StringValue"), name.to_string()));
        form.push((format!("{p}.MessageAttribute.1.Value.DataType"), "String".into()));
        form.push((format!("{p}.MessageAttribute.2.Name"), "Headers".into()));
        form.push((format!("{p}.MessageAttribute.2.Value.StringValue"), headers.to_string()));
        form.push((format!("{p}.MessageAttribute.2.Value.DataType"), "String".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v4_has_correct_shape() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4'); // version nibble
        let variant = u.chars().nth(19).unwrap();
        assert!(matches!(variant, '8' | '9' | 'a' | 'b'), "variant was {variant}");
    }

    #[test]
    fn utc_date_time_matches_known_epoch() {
        // 1783165501 s = 2026-07-04T11:45:01Z (the doc's 13:45:01 was local +02:00).
        let (d, t) = utc_date_time(1_783_165_501_000);
        assert_eq!(d, "2026-07-04");
        assert_eq!(t, "11:45:01");
    }

    #[test]
    fn utc_date_time_epoch_zero() {
        let (d, t) = utc_date_time(0);
        assert_eq!(d, "1970-01-01");
        assert_eq!(t, "00:00:00");
    }
}

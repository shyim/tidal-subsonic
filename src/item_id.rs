use std::fmt;
use std::str::FromStr;

/// A Subsonic item identifier, namespaced by the kind of entity it refers to.
///
/// On the wire these are strings with a two-letter prefix:
///   - `tr-<u64>`  a track
///   - `al-<u64>`  an album
///   - `ar-<u64>`  an artist
///   - `pl-<uuid>` a playlist (TIDAL playlists are UUID strings, not numeric)
///   - `mix-<id>` a TIDAL generated mix (surfaced as a playlist; id is a hex
///     string). Distinct prefix so `getPlaylist` routes mix ids to the
///     mix-tracks endpoint instead of the playlist one.
///
/// `FromStr` parses those prefixes and `Display` re-emits them, so the exact
/// wire format is centralised here instead of scattered `strip_prefix` /
/// `format!` call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemId {
    Track(u64),
    Album(u64),
    Artist(u64),
    Playlist(String),
    Mix(String),
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemId::Track(id) => write!(f, "tr-{}", id),
            ItemId::Album(id) => write!(f, "al-{}", id),
            ItemId::Artist(id) => write!(f, "ar-{}", id),
            ItemId::Playlist(uuid) => write!(f, "pl-{}", uuid),
            ItemId::Mix(id) => write!(f, "mix-{}", id),
        }
    }
}

impl FromStr for ItemId {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix("tr-") {
            return rest.parse().map(ItemId::Track).map_err(|_| ());
        }
        if let Some(rest) = s.strip_prefix("al-") {
            return rest.parse().map(ItemId::Album).map_err(|_| ());
        }
        if let Some(rest) = s.strip_prefix("ar-") {
            return rest.parse().map(ItemId::Artist).map_err(|_| ());
        }
        // `mix-` before `pl-`? No overlap, but check the longer/distinct prefix.
        if let Some(rest) = s.strip_prefix("mix-") {
            return Ok(ItemId::Mix(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("pl-") {
            return Ok(ItemId::Playlist(rest.to_string()));
        }
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders_each_prefix() {
        assert_eq!("tr-42".parse(), Ok(ItemId::Track(42)));
        assert_eq!("al-7".parse(), Ok(ItemId::Album(7)));
        assert_eq!("ar-9".parse(), Ok(ItemId::Artist(9)));
        assert_eq!("pl-abc-def".parse(), Ok(ItemId::Playlist("abc-def".to_string())));

        assert_eq!(ItemId::Track(42).to_string(), "tr-42");
        assert_eq!(ItemId::Album(7).to_string(), "al-7");
        assert_eq!(ItemId::Artist(9).to_string(), "ar-9");
        assert_eq!(ItemId::Playlist("abc-def".to_string()).to_string(), "pl-abc-def");
    }

    #[test]
    fn rejects_unknown_or_malformed() {
        assert_eq!("xx-1".parse::<ItemId>(), Err(()));
        assert_eq!("tr-notanumber".parse::<ItemId>(), Err(()));
        assert_eq!("no-prefix".parse::<ItemId>(), Err(()));
    }
}

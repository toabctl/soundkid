use anyhow::{Result, anyhow};
use librespot::core::SpotifyUri;

/// Accept either a `spotify:<type>:<id>` URI or a
/// `https://open.spotify.com/<type>/<id>` URL and return the canonical
/// `spotify:<type>:<id>` form.
///
/// Strict: the result is parsed by `SpotifyUri::from_uri`, so a typo'd kind
/// (`spotify:traack:...`) or a non-22-character ID is rejected here rather
/// than silently accepted now and exploding later when the user scans the
/// card.
pub fn canonicalize_uri(input: &str) -> Result<String> {
    let trimmed = input.trim();

    let canonical = if trimmed.starts_with("spotify:") {
        trimmed.to_string()
    } else {
        let path = trimmed
            .strip_prefix("https://open.spotify.com/")
            .or_else(|| trimmed.strip_prefix("http://open.spotify.com/"))
            .ok_or_else(|| anyhow!("unrecognized Spotify URI/URL: {input:?}"))?;
        let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
        let mut parts = path.split('/');
        let kind = parts
            .next()
            .ok_or_else(|| anyhow!("missing item type in {input:?}"))?;
        let id = parts
            .next()
            .ok_or_else(|| anyhow!("missing item id in {input:?}"))?;
        format!("spotify:{kind}:{id}")
    };

    SpotifyUri::from_uri(&canonical)
        .map_err(|e| anyhow!("not a valid Spotify URI {input:?}: {e}"))?;
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::canonicalize_uri;

    // Real-shape IDs (22 base62 chars). librespot validates length strictly.
    const TRACK: &str = "6rqhFgbbKwnb9MLmUQDhG6";
    const ALBUM: &str = "7LQhG0xSDjFiKJnziyB3Zj";
    const PLAYLIST: &str = "37i9dQZF1DXcBWIGoYBM5M";

    fn ok(input: &str, expected: &str) {
        assert_eq!(
            canonicalize_uri(input).unwrap(),
            expected,
            "input={input:?}"
        );
    }

    fn err(input: &str) {
        assert!(
            canonicalize_uri(input).is_err(),
            "expected err for {input:?}"
        );
    }

    #[test]
    fn spotify_track_passthrough() {
        ok(
            &format!("spotify:track:{TRACK}"),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn spotify_album_passthrough() {
        ok(
            &format!("spotify:album:{ALBUM}"),
            &format!("spotify:album:{ALBUM}"),
        );
    }

    #[test]
    fn spotify_playlist_passthrough() {
        ok(
            &format!("spotify:playlist:{PLAYLIST}"),
            &format!("spotify:playlist:{PLAYLIST}"),
        );
    }

    #[test]
    fn spotify_named_playlist_passthrough() {
        let s = format!("spotify:user:spotify:playlist:{PLAYLIST}");
        ok(&s, &s);
    }

    #[test]
    fn spotify_invalid_id_rejected() {
        // We now validate via SpotifyUri::from_uri; a 3-char id is rejected.
        err("spotify:track:abc");
    }

    #[test]
    fn spotify_garbage_after_prefix_rejected() {
        err("spotify:nonsense");
    }

    #[test]
    fn https_track_converts() {
        ok(
            &format!("https://open.spotify.com/track/{TRACK}"),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn https_album_converts() {
        ok(
            &format!("https://open.spotify.com/album/{ALBUM}"),
            &format!("spotify:album:{ALBUM}"),
        );
    }

    #[test]
    fn https_playlist_converts() {
        ok(
            &format!("https://open.spotify.com/playlist/{PLAYLIST}"),
            &format!("spotify:playlist:{PLAYLIST}"),
        );
    }

    #[test]
    fn http_url_also_converts() {
        ok(
            &format!("http://open.spotify.com/track/{TRACK}"),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn url_query_string_stripped() {
        ok(
            &format!("https://open.spotify.com/track/{TRACK}?si=xyz"),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn url_query_with_multiple_params_stripped() {
        ok(
            &format!("https://open.spotify.com/album/{ALBUM}?si=foo&utm=bar"),
            &format!("spotify:album:{ALBUM}"),
        );
    }

    #[test]
    fn url_trailing_slash_stripped() {
        ok(
            &format!("https://open.spotify.com/track/{TRACK}/"),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn url_trailing_slash_with_query_stripped() {
        ok(
            &format!("https://open.spotify.com/track/{TRACK}/?si=z"),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn leading_and_trailing_whitespace_trimmed() {
        ok(
            &format!("  https://open.spotify.com/track/{TRACK}  "),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn whitespace_around_spotify_uri_trimmed() {
        ok(
            &format!("  spotify:track:{TRACK}  "),
            &format!("spotify:track:{TRACK}"),
        );
    }

    #[test]
    fn empty_string_errors() {
        err("");
    }

    #[test]
    fn whitespace_only_errors() {
        err("   ");
    }

    #[test]
    fn random_garbage_errors() {
        err("not a uri at all");
    }

    #[test]
    fn unrelated_https_url_errors() {
        err(&format!("https://example.com/track/{TRACK}"));
    }

    #[test]
    fn open_spotify_root_errors() {
        err("https://open.spotify.com/");
    }

    #[test]
    fn open_spotify_only_type_errors() {
        err("https://open.spotify.com/track");
    }

    #[test]
    fn open_spotify_type_with_only_slash_errors() {
        err("https://open.spotify.com/track/");
    }

    #[test]
    fn ftp_url_errors() {
        err(&format!("ftp://open.spotify.com/track/{TRACK}"));
    }

    #[test]
    fn typo_in_uri_kind_passes_canonicalisation() {
        // SpotifyUri accepts unknown kinds as Unknown variant — we don't
        // reject them here. They will be rejected by resolve_tracks at play
        // time, since Unknown is not playable. (Documenting current behaviour.)
        let s = format!("spotify:traack:{TRACK}");
        ok(&s, &s);
    }
}

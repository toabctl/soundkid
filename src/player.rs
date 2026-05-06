use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use librespot::core::{
    SpotifyUri, authentication::Credentials, cache::Cache, config::SessionConfig, session::Session,
};
use librespot::metadata::{Album, Metadata, Playlist};
use librespot::playback::{
    audio_backend,
    config::{AudioFormat, PlayerConfig},
    mixer::NoOpVolume,
    player::Player,
};
use librespot_oauth::OAuthClientBuilder;
use log::{info, warn};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;

use crate::config::ConfigSpotify;

/// OAuth scopes mirroring what the upstream librespot binary requests.
/// A subset would suffice for playback, but matching upstream avoids surprises.
const OAUTH_SCOPES: &[&str] = &["streaming", "user-read-playback-state"];

/// Commands sent from the input handler to the player task.
#[derive(Debug, Clone)]
enum Command {
    Play(String),
    Stop,
    Pause,
    Resume,
}

/// Bounded queue depth: in normal operation we never exceed one or two
/// in-flight commands, so this is generous. A full channel means the player
/// task is wedged, and `send().await` will park the input loop until it isn't.
const COMMAND_QUEUE_DEPTH: usize = 8;

/// Cheap, clonable handle to the background player task.
#[derive(Clone)]
pub struct SpotifyPlayer {
    tx: Sender<Command>,
}

impl SpotifyPlayer {
    /// Connect to Spotify and spawn the background player task.
    ///
    /// Returns a clonable `SpotifyPlayer` handle for sending commands and the
    /// `JoinHandle` of the background task. Callers should watch the handle so
    /// that a panic or unexpected return takes down the process.
    pub async fn new(spotify: &ConfigSpotify) -> Result<(Self, JoinHandle<()>)> {
        let mut session_config = SessionConfig::default();
        if let Some(client_id) = &spotify.client_id {
            session_config.client_id = client_id.clone();
        }

        let cache = Cache::new(Some(&spotify.cache_dir), None, None, None)
            .with_context(|| format!("failed to open cache at {:?}", spotify.cache_dir))?;

        let credentials = match cache.credentials() {
            Some(c) => {
                info!("Using cached credentials from {:?}", spotify.cache_dir);
                c
            }
            None => {
                info!("No cached credentials found, starting OAuth login");
                Credentials::with_access_token(
                    oauth_login(&session_config.client_id).await?.access_token,
                )
            }
        };

        info!("Connecting to Spotify ...");
        let session = Session::new(session_config, Some(cache));
        session
            .connect(credentials, true)
            .await
            .context("failed to connect to Spotify")?;
        info!("Connected.");

        let backend = audio_backend::find(None).ok_or_else(|| anyhow!("no audio backend"))?;
        let player = Player::new(
            PlayerConfig::default(),
            session.clone(),
            Box::new(NoOpVolume),
            move || backend(None, AudioFormat::default()),
        );

        let (tx, rx) = mpsc::channel(COMMAND_QUEUE_DEPTH);
        let join = tokio::spawn(player_task(session, player, rx));

        Ok((Self { tx }, join))
    }

    pub async fn play(&self, uri: String) -> Result<()> {
        self.send(Command::Play(uri)).await
    }

    pub async fn stop(&self) -> Result<()> {
        self.send(Command::Stop).await
    }

    pub async fn pause(&self) -> Result<()> {
        self.send(Command::Pause).await
    }

    pub async fn resume(&self) -> Result<()> {
        self.send(Command::Resume).await
    }

    async fn send(&self, cmd: Command) -> Result<()> {
        self.tx
            .send(cmd)
            .await
            .map_err(|_| anyhow!("player task is no longer running"))
    }
}

async fn oauth_login(client_id: &str) -> Result<librespot_oauth::OAuthToken> {
    let client = OAuthClientBuilder::new(client_id, "http://127.0.0.1:8898/login", OAUTH_SCOPES.to_vec())
        .open_in_browser()
        .build()
        .map_err(|e| anyhow!("failed to build OAuth client: {e}"))?;
    client
        .get_access_token_async()
        .await
        .map_err(|e| anyhow!("failed to obtain Spotify access token: {e}"))
}

/// Player state machine. `Idle` waits for the next command, `Playing`
/// races track completion against incoming commands so a new card scan
/// (or pause/resume/stop) takes effect immediately.
enum State {
    Idle,
    Playing { queue: Vec<SpotifyUri>, idx: usize },
}

async fn player_task(session: Session, player: Arc<Player>, mut rx: Receiver<Command>) {
    let mut state = State::Idle;
    loop {
        state = match state {
            State::Idle => match rx.recv().await {
                Some(cmd) => apply_command(cmd, &session, &player, State::Idle).await,
                None => return,
            },
            State::Playing { queue, idx } if idx >= queue.len() => State::Idle,
            State::Playing { queue, idx } => {
                let track_uri = queue[idx].clone();
                info!("Playing {track_uri:?}");
                player.load(track_uri, true, 0);
                tokio::select! {
                    _ = player.await_end_of_track() => {
                        State::Playing { queue, idx: idx + 1 }
                    }
                    cmd = rx.recv() => match cmd {
                        Some(cmd) => apply_command(
                            cmd, &session, &player, State::Playing { queue, idx },
                        ).await,
                        None => return,
                    }
                }
            }
        };
    }
}

async fn apply_command(
    cmd: Command,
    session: &Session,
    player: &Arc<Player>,
    state: State,
) -> State {
    match cmd {
        Command::Play(uri) => {
            if matches!(state, State::Playing { .. }) {
                player.stop();
            }
            match resolve_tracks(session, &uri).await {
                Ok(queue) if queue.is_empty() => {
                    warn!("URI {uri:?} resolved to no playable tracks");
                    State::Idle
                }
                Ok(queue) => State::Playing { queue, idx: 0 },
                Err(e) => {
                    warn!("could not resolve {uri:?}: {e:#}");
                    State::Idle
                }
            }
        }
        Command::Stop => {
            player.stop();
            State::Idle
        }
        Command::Pause => {
            player.pause();
            state
        }
        Command::Resume => {
            player.play();
            state
        }
    }
}

/// Turn a Spotify URI or open.spotify.com URL into a flat list of track URIs.
async fn resolve_tracks(session: &Session, raw: &str) -> Result<Vec<SpotifyUri>> {
    let canonical = canonicalize_uri(raw)?;
    let uri = SpotifyUri::from_uri(&canonical)
        .map_err(|e| anyhow!("not a valid Spotify URI {raw:?}: {e}"))?;

    Ok(match &uri {
        SpotifyUri::Track { .. } => vec![uri],
        SpotifyUri::Album { .. } => Album::get(session, &uri)
            .await
            .map_err(|e| anyhow!("Album::get failed: {e}"))?
            .tracks()
            .cloned()
            .collect(),
        SpotifyUri::Playlist { .. } => Playlist::get(session, &uri)
            .await
            .map_err(|e| anyhow!("Playlist::get failed: {e}"))?
            .tracks()
            .cloned()
            .collect(),
        other => {
            bail!("URI type {} not supported for playback", other.item_type());
        }
    })
}

/// Accept either a `spotify:track:...` URI or a `https://open.spotify.com/<type>/<id>...` URL
/// and return the canonical `spotify:<type>:<id>` form.
fn canonicalize_uri(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("spotify:") {
        return Ok(trimmed.to_string());
    }

    // Strip scheme + host so we can split on '/'.
    let path = trimmed
        .strip_prefix("https://open.spotify.com/")
        .or_else(|| trimmed.strip_prefix("http://open.spotify.com/"))
        .ok_or_else(|| anyhow!("unrecognized Spotify URI/URL: {input:?}"))?;

    // Drop any query string (`?si=...`) and trailing slashes.
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    let mut parts = path.split('/');
    let kind = parts
        .next()
        .ok_or_else(|| anyhow!("missing item type in {input:?}"))?;
    let id = parts
        .next()
        .ok_or_else(|| anyhow!("missing item id in {input:?}"))?;

    Ok(format!("spotify:{kind}:{id}"))
}

#[cfg(test)]
mod tests {
    use super::canonicalize_uri;

    fn ok(input: &str, expected: &str) {
        assert_eq!(canonicalize_uri(input).unwrap(), expected, "input={input:?}");
    }

    fn err(input: &str) {
        assert!(canonicalize_uri(input).is_err(), "expected err for {input:?}");
    }

    // ----- spotify: passthrough -----------------------------------------

    #[test]
    fn spotify_track_passthrough() {
        ok("spotify:track:abc", "spotify:track:abc");
    }

    #[test]
    fn spotify_album_passthrough() {
        ok("spotify:album:xyz", "spotify:album:xyz");
    }

    #[test]
    fn spotify_playlist_passthrough() {
        ok("spotify:playlist:42", "spotify:playlist:42");
    }

    #[test]
    fn spotify_named_playlist_passthrough() {
        // The `spotify:user:<user>:playlist:<id>` form is preserved verbatim;
        // SpotifyUri::from_uri handles the named variant downstream.
        ok(
            "spotify:user:spotify:playlist:37i9dQZF1DWSw8liJZcPOI",
            "spotify:user:spotify:playlist:37i9dQZF1DWSw8liJZcPOI",
        );
    }

    #[test]
    fn spotify_passthrough_is_not_validated() {
        // Documented behaviour: we hand off validation to SpotifyUri::from_uri.
        ok("spotify:nonsense", "spotify:nonsense");
    }

    // ----- https URL conversion -----------------------------------------

    #[test]
    fn https_track_converts() {
        ok("https://open.spotify.com/track/abc", "spotify:track:abc");
    }

    #[test]
    fn https_album_converts() {
        ok("https://open.spotify.com/album/xyz", "spotify:album:xyz");
    }

    #[test]
    fn https_playlist_converts() {
        ok("https://open.spotify.com/playlist/p1", "spotify:playlist:p1");
    }

    #[test]
    fn http_url_also_converts() {
        ok("http://open.spotify.com/track/abc", "spotify:track:abc");
    }

    #[test]
    fn url_query_string_stripped() {
        ok(
            "https://open.spotify.com/track/abc?si=xyz",
            "spotify:track:abc",
        );
    }

    #[test]
    fn url_query_with_multiple_params_stripped() {
        ok(
            "https://open.spotify.com/album/xyz?si=foo&utm=bar",
            "spotify:album:xyz",
        );
    }

    #[test]
    fn url_trailing_slash_stripped() {
        ok("https://open.spotify.com/track/abc/", "spotify:track:abc");
    }

    #[test]
    fn url_trailing_slash_with_query_stripped() {
        ok(
            "https://open.spotify.com/track/abc/?si=z",
            "spotify:track:abc",
        );
    }

    #[test]
    fn leading_and_trailing_whitespace_trimmed() {
        ok(
            "  https://open.spotify.com/track/abc  ",
            "spotify:track:abc",
        );
    }

    #[test]
    fn whitespace_around_spotify_uri_trimmed() {
        ok("  spotify:track:abc  ", "spotify:track:abc");
    }

    // ----- error cases --------------------------------------------------

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
        err("https://example.com/track/abc");
    }

    #[test]
    fn open_spotify_root_errors() {
        // No type or id.
        err("https://open.spotify.com/");
    }

    #[test]
    fn open_spotify_only_type_errors() {
        // No id after the type.
        err("https://open.spotify.com/track");
    }

    #[test]
    fn open_spotify_type_with_only_slash_errors() {
        // After trim_end_matches('/') and split, kind="track" but id is missing.
        err("https://open.spotify.com/track/");
    }

    #[test]
    fn ftp_url_errors() {
        err("ftp://open.spotify.com/track/abc");
    }
}

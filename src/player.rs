use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use librespot::core::{
    SpotifyUri, authentication::Credentials, cache::Cache, config::SessionConfig, session::Session,
};
use librespot::metadata::{Album, Metadata, Playlist, Track};
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

async fn player_task(session: Session, player: Arc<Player>, mut rx: Receiver<Command>) {
    let mut queue: Vec<SpotifyUri> = Vec::new();
    let mut idx: usize = 0;

    loop {
        // Idle: wait for the next command.
        if idx >= queue.len() {
            match rx.recv().await {
                Some(cmd) => match handle_idle(cmd, &session, &player, &mut queue, &mut idx).await {
                    Ok(()) => {}
                    Err(e) => warn!("player command failed: {e:#}"),
                },
                None => return, // sender dropped
            }
            continue;
        }

        // Active: load the next track and race its completion against new commands.
        let track_uri = queue[idx].clone();
        match Track::get(&session, &track_uri).await {
            Ok(t) => info!("Playing track '{}' ({:?})", t.name, track_uri),
            Err(e) => warn!("could not fetch metadata for {track_uri:?}: {e}"),
        }
        player.load(track_uri, true, 0);

        tokio::select! {
            _ = player.await_end_of_track() => {
                idx += 1;
            }
            cmd = rx.recv() => match cmd {
                Some(cmd) => match handle_active(cmd, &session, &player, &mut queue, &mut idx).await {
                    Ok(()) => {}
                    Err(e) => warn!("player command failed: {e:#}"),
                },
                None => return,
            }
        }
    }
}

async fn handle_idle(
    cmd: Command,
    session: &Session,
    player: &Arc<Player>,
    queue: &mut Vec<SpotifyUri>,
    idx: &mut usize,
) -> Result<()> {
    match cmd {
        Command::Play(uri) => {
            *queue = resolve_tracks(session, &uri).await?;
            *idx = 0;
            if queue.is_empty() {
                warn!("URI {uri:?} resolved to no playable tracks");
            }
        }
        Command::Pause | Command::Resume => {
            // No active playback to pause/resume — ignore.
        }
        Command::Stop => {
            player.stop();
            queue.clear();
            *idx = 0;
        }
    }
    Ok(())
}

async fn handle_active(
    cmd: Command,
    session: &Session,
    player: &Arc<Player>,
    queue: &mut Vec<SpotifyUri>,
    idx: &mut usize,
) -> Result<()> {
    match cmd {
        Command::Play(uri) => {
            player.stop();
            *queue = resolve_tracks(session, &uri).await?;
            *idx = 0;
        }
        Command::Stop => {
            player.stop();
            queue.clear();
            *idx = 0;
        }
        Command::Pause => player.pause(),
        Command::Resume => player.play(),
    }
    Ok(())
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

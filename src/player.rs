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
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::{info, warn};

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

/// The contract the dispatch loop relies on. Production uses `SpotifyPlayer`;
/// tests substitute a fake. `Send + Sync + 'static` is what `tokio::spawn`
/// demands of anything captured by a spawned task.
pub trait PlayerControl: Clone + Send + Sync + 'static {
    fn play(&self, uri: String) -> impl std::future::Future<Output = Result<()>> + Send;
    fn stop(&self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn pause(&self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn resume(&self) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Cheap, clonable handle to the background player task.
#[derive(Clone)]
pub struct SpotifyPlayer {
    tx: Sender<Command>,
}

impl PlayerControl for SpotifyPlayer {
    async fn play(&self, uri: String) -> Result<()> {
        self.send(Command::Play(uri)).await
    }

    async fn stop(&self) -> Result<()> {
        self.send(Command::Stop).await
    }

    async fn pause(&self) -> Result<()> {
        self.send(Command::Pause).await
    }

    async fn resume(&self) -> Result<()> {
        self.send(Command::Resume).await
    }
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

    async fn send(&self, cmd: Command) -> Result<()> {
        self.tx
            .send(cmd)
            .await
            .map_err(|_| anyhow!("player task is no longer running"))
    }
}

async fn oauth_login(client_id: &str) -> Result<librespot_oauth::OAuthToken> {
    let client = OAuthClientBuilder::new(
        client_id,
        "http://127.0.0.1:8898/login",
        OAUTH_SCOPES.to_vec(),
    )
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

/// Turn a Spotify URI into a flat list of track URIs. The URI is expected
/// to already be in canonical `spotify:<type>:<id>` form (Action::Play
/// canonicalises at config load).
async fn resolve_tracks(session: &Session, canonical: &str) -> Result<Vec<SpotifyUri>> {
    let uri = SpotifyUri::from_uri(canonical)
        .map_err(|e| anyhow!("not a valid Spotify URI {canonical:?}: {e}"))?;

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

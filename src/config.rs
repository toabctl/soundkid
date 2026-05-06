use anyhow::{Context, Result, anyhow};
use log::{info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

fn default_alsa_control() -> String {
    "Master".to_string()
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/var/cache"))
        .join("soundkid")
}

/// What soundkid should do when a configured input fires. Parsed from the
/// raw YAML string at config load, so a typo (`VOLUME_INCREASS`) is rejected
/// at startup rather than silently misrouted as a Spotify URI later.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum Action {
    VolumeIncrease,
    VolumeDecrease,
    Pause,
    Resume,
    /// A Spotify URI (`spotify:track:...`) or an `open.spotify.com` URL.
    /// Stored verbatim; the player is responsible for canonicalisation.
    Play(String),
}

impl FromStr for Action {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "VOLUME_INCREASE" => Ok(Action::VolumeIncrease),
            "VOLUME_DECREASE" => Ok(Action::VolumeDecrease),
            "PAUSE" => Ok(Action::Pause),
            "RESUME" => Ok(Action::Resume),
            other if other.starts_with("spotify:")
                || other.starts_with("https://open.spotify.com/")
                || other.starts_with("http://open.spotify.com/") =>
            {
                Ok(Action::Play(other.to_string()))
            }
            other => Err(anyhow!(
                "unknown action {other:?}: expected VOLUME_INCREASE, VOLUME_DECREASE, \
                 PAUSE, RESUME, or a spotify: URI / open.spotify.com URL"
            )),
        }
    }
}

impl TryFrom<String> for Action {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Action::from_str(&value)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default)]
    pub gpio: HashMap<String, HashMap<u32, Action>>,
    #[serde(default)]
    pub input: HashMap<String, HashMap<String, Action>>,
    pub alsa: ConfigAlsa,
    pub spotify: ConfigSpotify,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ConfigSpotify {
    /// Directory used to persist credentials after the one-time OAuth login,
    /// so subsequent starts can connect headlessly.
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
    /// Optional Spotify client_id. Defaults to librespot's built-in keymaster id.
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ConfigAlsa {
    #[serde(default = "default_alsa_control")]
    pub control: String,
}

impl Config {
    pub async fn load() -> Result<Self> {
        let candidates = [
            dirs::home_dir().map(|h| h.join(".soundkid.conf")),
            Some(PathBuf::from("/etc/soundkid.conf")),
        ];

        let mut last_err: Option<anyhow::Error> = None;
        for path in candidates.into_iter().flatten() {
            info!("Trying to read config file {path:?}");
            match tokio::fs::read_to_string(&path).await {
                Ok(contents) => match serde_yaml_ng::from_str::<Config>(&contents) {
                    Ok(cfg) => return Ok(cfg),
                    Err(e) => {
                        warn!("Unable to parse yaml from {path:?}: {e}");
                        last_err = Some(anyhow!("parse error in {}: {e}", path.display()));
                    }
                },
                Err(e) => {
                    info!("Unable to read config file {path:?}: {e}");
                    last_err = Some(anyhow!("read error for {}: {e}", path.display()));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("no config file found")))
            .context("unable to load any soundkid config (~/.soundkid.conf or /etc/soundkid.conf)")
    }
}

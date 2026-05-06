use anyhow::{Context, Result, anyhow};
use log::{info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

fn default_alsa_control() -> String {
    "Master".to_string()
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/var/cache"))
        .join("soundkid")
}

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default)]
    pub gpio: HashMap<String, HashMap<u32, String>>,
    #[serde(default)]
    pub input: HashMap<String, HashMap<String, String>>,
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

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use thiserror::Error;
use tracing::{info, warn};

use crate::uri::{UriError, canonicalize_uri};

#[derive(Debug, Error)]
pub enum ActionParseError {
    #[error(
        "unknown action {0:?}: expected VOLUME_INCREASE, VOLUME_DECREASE, PAUSE, RESUME, \
         or a spotify: URI / open.spotify.com URL"
    )]
    UnknownKeyword(String),
    #[error(transparent)]
    InvalidUri(#[from] UriError),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read error for {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse error in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },
    #[error(
        "no readable config file (tried ~/.soundkid.conf and /etc/soundkid.conf, or the paths \
         passed to load_from)"
    )]
    NoCandidate,
}

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
    /// A Spotify URI in canonical `spotify:<type>:<id>` form. URLs of the
    /// form `https://open.spotify.com/...` are normalised to this shape at
    /// parse time, so by the time the player sees this it is already valid.
    Play(String),
}

impl FromStr for Action {
    type Err = ActionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "VOLUME_INCREASE" => Ok(Action::VolumeIncrease),
            "VOLUME_DECREASE" => Ok(Action::VolumeDecrease),
            "PAUSE" => Ok(Action::Pause),
            "RESUME" => Ok(Action::Resume),
            other
                if other.starts_with("spotify:")
                    || other.starts_with("https://open.spotify.com/")
                    || other.starts_with("http://open.spotify.com/") =>
            {
                Ok(Action::Play(canonicalize_uri(other)?))
            }
            other => Err(ActionParseError::UnknownKeyword(other.to_string())),
        }
    }
}

impl TryFrom<String> for Action {
    type Error = ActionParseError;
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
    /// Load from the standard candidate paths: `~/.soundkid.conf` first, then
    /// `/etc/soundkid.conf`. The first one that reads and parses successfully
    /// wins; later candidates only become relevant if earlier ones fail.
    pub async fn load() -> Result<Self, ConfigError> {
        Self::load_from(default_candidates()).await
    }

    /// Load from an explicit list of candidate paths, in order. The first
    /// path that reads and parses cleanly wins; missing or unparseable files
    /// are skipped. Errors from later candidates suppress errors from earlier
    /// ones, so the last error is reported when nothing succeeds.
    ///
    /// Exposed mainly for tests; production callers should use [`Config::load`].
    pub async fn load_from<I>(candidates: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let mut last_err: Option<ConfigError> = None;
        for path in candidates {
            info!("Trying to read config file {path:?}");
            match tokio::fs::read_to_string(&path).await {
                Ok(contents) => match serde_yaml_ng::from_str::<Config>(&contents) {
                    Ok(cfg) => return Ok(cfg),
                    Err(source) => {
                        warn!("Unable to parse yaml from {path:?}: {source}");
                        last_err = Some(ConfigError::Parse { path, source });
                    }
                },
                Err(source) => {
                    info!("Unable to read config file {path:?}: {source}");
                    last_err = Some(ConfigError::Read { path, source });
                }
            }
        }
        Err(last_err.unwrap_or(ConfigError::NoCandidate))
    }
}

fn default_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".soundkid.conf"));
    }
    out.push(PathBuf::from("/etc/soundkid.conf"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ----- Action::from_str ----------------------------------------------

    #[test]
    fn action_volume_increase() {
        assert_eq!(
            Action::from_str("VOLUME_INCREASE").unwrap(),
            Action::VolumeIncrease
        );
    }

    #[test]
    fn action_volume_decrease() {
        assert_eq!(
            Action::from_str("VOLUME_DECREASE").unwrap(),
            Action::VolumeDecrease
        );
    }

    #[test]
    fn action_pause_resume() {
        assert_eq!(Action::from_str("PAUSE").unwrap(), Action::Pause);
        assert_eq!(Action::from_str("RESUME").unwrap(), Action::Resume);
    }

    // Real-shape IDs (22 base62 chars). librespot validates length strictly.
    const TRACK_ID: &str = "6rqhFgbbKwnb9MLmUQDhG6";
    const ALBUM_ID: &str = "7LQhG0xSDjFiKJnziyB3Zj";

    #[test]
    fn action_play_spotify_uri_passthrough() {
        let s = format!("spotify:track:{TRACK_ID}");
        assert_eq!(Action::from_str(&s).unwrap(), Action::Play(s));
    }

    #[test]
    fn action_play_https_url_canonicalised() {
        let url = format!("https://open.spotify.com/album/{ALBUM_ID}");
        assert_eq!(
            Action::from_str(&url).unwrap(),
            Action::Play(format!("spotify:album:{ALBUM_ID}"))
        );
    }

    #[test]
    fn action_play_http_url_canonicalised() {
        let url = format!("http://open.spotify.com/track/{TRACK_ID}");
        assert_eq!(
            Action::from_str(&url).unwrap(),
            Action::Play(format!("spotify:track:{TRACK_ID}"))
        );
    }

    #[test]
    fn action_play_url_with_query_string_canonicalised() {
        let url = format!("https://open.spotify.com/album/{ALBUM_ID}?si=xyz");
        assert_eq!(
            Action::from_str(&url).unwrap(),
            Action::Play(format!("spotify:album:{ALBUM_ID}"))
        );
    }

    #[test]
    fn action_play_invalid_spotify_id_rejected_at_parse() {
        // Short/invalid id is rejected by canonicalize_uri at config load,
        // not at first card scan.
        assert!(Action::from_str("spotify:track:abc").is_err());
    }

    #[test]
    fn action_play_invalid_url_id_rejected_at_parse() {
        assert!(Action::from_str("https://open.spotify.com/track/abc").is_err());
    }

    #[test]
    fn action_typo_keyword_rejected() {
        // PAUSE → PAUSED is a typo we want caught at startup.
        let err = Action::from_str("PAUSED").unwrap_err().to_string();
        assert!(err.contains("PAUSED"), "error should name the bad value");
    }

    #[test]
    fn action_lowercase_keyword_rejected() {
        // Keywords are uppercase by convention; lowercase is a mistake.
        assert!(Action::from_str("pause").is_err());
        assert!(Action::from_str("Pause").is_err());
    }

    #[test]
    fn action_empty_rejected() {
        assert!(Action::from_str("").is_err());
    }

    #[test]
    fn action_random_string_rejected() {
        assert!(Action::from_str("just-some-text").is_err());
    }

    #[test]
    fn action_wrong_host_rejected() {
        // Looks like a URL but isn't open.spotify.com.
        assert!(Action::from_str("https://example.com/track/abc").is_err());
    }

    #[test]
    fn action_spotify_garbage_after_prefix_rejected() {
        // Body validation happens at config load via SpotifyUri::from_uri.
        assert!(Action::from_str("spotify:nonsense").is_err());
    }

    // ----- Config parsing ------------------------------------------------

    fn parse(yaml: &str) -> Result<Config, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(yaml)
    }

    #[test]
    fn config_minimal_uses_defaults() {
        let cfg = parse(
            r#"
alsa: {}
spotify: {}
"#,
        )
        .unwrap();
        assert_eq!(cfg.alsa.control, "Master");
        assert!(cfg.spotify.client_id.is_none());
        assert!(cfg.input.is_empty());
        assert!(cfg.gpio.is_empty());
    }

    #[test]
    fn config_full_parses() {
        let yaml = format!(
            r#"
alsa: {{ control: "SoftMaster" }}
spotify:
  client_id: my-id
input:
  /dev/input/event0:
    "12345": "spotify:track:{TRACK_ID}"
    "VOL": "VOLUME_INCREASE"
gpio:
  /dev/gpiochip0:
    17: PAUSE
    27: RESUME
"#
        );
        let cfg = parse(&yaml).unwrap();
        assert_eq!(cfg.alsa.control, "SoftMaster");
        assert_eq!(cfg.spotify.client_id.as_deref(), Some("my-id"));
        let evdev = &cfg.input["/dev/input/event0"];
        assert_eq!(
            evdev["12345"],
            Action::Play(format!("spotify:track:{TRACK_ID}"))
        );
        assert_eq!(evdev["VOL"], Action::VolumeIncrease);
        let gpio = &cfg.gpio["/dev/gpiochip0"];
        assert_eq!(gpio[&17u32], Action::Pause);
        assert_eq!(gpio[&27u32], Action::Resume);
    }

    #[test]
    fn config_invalid_action_in_yaml_fails() {
        let err = parse(
            r#"
alsa: {}
spotify: {}
input:
  /dev/input/event0:
    "12345": "VOLUME_INCREASS"
"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("VOLUME_INCREASS"),
            "error should mention the typo: {err}"
        );
    }

    #[test]
    fn config_missing_alsa_section_fails() {
        // alsa is required (no #[serde(default)] on the field).
        assert!(parse("spotify: {}").is_err());
    }

    #[test]
    fn config_missing_spotify_section_fails() {
        assert!(parse("alsa: {}").is_err());
    }

    #[test]
    fn config_invalid_yaml_fails() {
        assert!(parse(": this is :: not yaml").is_err());
    }

    #[test]
    fn config_alsa_default_only() {
        let cfg = parse(
            r#"
alsa: {}
spotify: {}
"#,
        )
        .unwrap();
        // default_alsa_control() returns "Master".
        assert_eq!(cfg.alsa.control, "Master");
    }

    // ----- Config::load_from --------------------------------------------

    fn write_temp(yaml: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f
    }

    const VALID_YAML: &str = r#"
alsa: { control: "Master" }
spotify: {}
"#;

    const VALID_YAML_2: &str = r#"
alsa: { control: "SoftMaster" }
spotify: {}
"#;

    #[tokio::test]
    async fn load_from_first_match_wins() {
        let first = write_temp(VALID_YAML);
        let second = write_temp(VALID_YAML_2);
        let cfg = Config::load_from(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ])
        .await
        .unwrap();
        assert_eq!(cfg.alsa.control, "Master");
    }

    #[tokio::test]
    async fn load_from_skips_missing_then_parses() {
        let real = write_temp(VALID_YAML);
        let cfg = Config::load_from(vec![
            PathBuf::from("/nonexistent/soundkid.conf"),
            real.path().to_path_buf(),
        ])
        .await
        .unwrap();
        assert_eq!(cfg.alsa.control, "Master");
    }

    #[tokio::test]
    async fn load_from_skips_invalid_yaml_then_parses() {
        let bad = write_temp(": broken yaml ::");
        let good = write_temp(VALID_YAML_2);
        let cfg = Config::load_from(vec![bad.path().to_path_buf(), good.path().to_path_buf()])
            .await
            .unwrap();
        assert_eq!(cfg.alsa.control, "SoftMaster");
    }

    #[tokio::test]
    async fn load_from_all_missing_fails() {
        let res = Config::load_from(vec![
            PathBuf::from("/nonexistent/a.conf"),
            PathBuf::from("/nonexistent/b.conf"),
        ])
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn load_from_empty_candidates_fails() {
        let res = Config::load_from(Vec::<PathBuf>::new()).await;
        assert!(matches!(res, Err(ConfigError::NoCandidate)));
    }

    #[tokio::test]
    async fn load_from_all_invalid_fails_with_last_parse_error() {
        let bad1 = write_temp(": one ::");
        let bad2 = write_temp("alsa: {}\nspotify: {}\ninput:\n  dev:\n    \"x\": \"BAD_ACTION\"\n");
        let err = Config::load_from(vec![bad1.path().to_path_buf(), bad2.path().to_path_buf()])
            .await
            .unwrap_err();
        // Last error should be the second one (the "BAD_ACTION" one).
        assert!(err.to_string().contains("BAD_ACTION"));
    }
}

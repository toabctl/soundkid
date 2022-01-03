/// The config module that handles the soundkid configuration
extern crate dirs;

use log::{info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn default_alsa_control() -> String {
    return "Master".to_string();
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
    pub username: String,
    pub password: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ConfigAlsa {
    #[serde(default = "default_alsa_control")]
    pub control: String,
}

impl Config {
    pub fn new() -> Config {
        let mut config_home = PathBuf::new();
        config_home.push(dirs::home_dir().unwrap());
        config_home.push(".soundkid.conf");

        let config_global = PathBuf::from("/etc/soundkid.conf");

        for c in [&config_home, &config_global].iter() {
            info!("Trying to read config file {:?}", c);
            let yaml_content = fs::read_to_string(c.as_path());
            let yaml_content = match yaml_content {
                Ok(content) => content,
                Err(e) => {
                    info!("Unable to read config file {:?}: {}", c, e.to_string());
                    continue;
                }
            };
            let yaml_config = serde_yaml::from_str(yaml_content.as_str());
            match yaml_config {
                Ok(config) => {
                    return config;
                }
                Err(e) => {
                    warn!("Unable to parse yaml from file {:?}: {}", c, e.to_string());
                    continue;
                }
            };
        }

        panic!("Unable to read any config file. ciao");
    }
}

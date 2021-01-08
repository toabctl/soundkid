/// The config module that handles the soundkid configuration
extern crate dirs;

use log::info;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn default_alsa_control() -> String {
    return "Master".to_string();
}

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub gpio: HashMap<String, HashMap<u32, String>>,
    #[serde(default)]
    pub input: HashMap<String, HashMap<String, String>>,
    pub alsa: ConfigAlsa,
    pub spotify: ConfigSpotify,
}

#[derive(Deserialize, Debug)]
pub struct ConfigSpotify {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize, Debug)]
pub struct ConfigAlsa {
    #[serde(default = "default_alsa_control")]
    pub control: String,
}

impl Config {
    pub fn new() -> Config {
        let mut config_file_path = PathBuf::new();
        config_file_path.push(dirs::home_dir().unwrap());
        config_file_path.push(".soundkid.conf");
        info!("Trying to read config file...");
        let yaml_content = fs::read_to_string(config_file_path.as_path()).unwrap();
        let yaml_config: Config = serde_yaml::from_str(yaml_content.as_str()).unwrap();
        return yaml_config;
    }
}

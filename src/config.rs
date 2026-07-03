use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub tidal: TidalConfig,
    pub server: ServerConfig,
    pub subsonic: SubsonicAuth,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TidalConfig {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub user_id: Option<u64>,
    #[serde(default = "default_country_code")]
    pub country_code: String,
    #[serde(default = "default_max_quality")]
    pub max_quality: String,
}

fn default_country_code() -> String {
    "US".to_string()
}

fn default_max_quality() -> String {
    "HI_RES_LOSSLESS".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    4533
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubsonicAuth {
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default = "default_password")]
    pub password: String,
}

fn default_username() -> String {
    "tidal".to_string()
}

fn default_password() -> String {
    "tidal".to_string()
}

impl Config {
    pub fn load_or_create() -> Self {
        let path = config_path();
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(config) = toml::from_str(&contents) {
                return config;
            }
        }

        let config = Config {
            tidal: TidalConfig {
                client_id: String::new(),
                client_secret: None,
                access_token: None,
                refresh_token: None,
                user_id: None,
                country_code: default_country_code(),
                max_quality: default_max_quality(),
            },
            server: ServerConfig {
                host: default_host(),
                port: default_port(),
            },
            subsonic: SubsonicAuth {
                username: default_username(),
                password: default_password(),
            },
        };

        config.save().ok();
        config
    }

    pub fn save(&self) -> Result<(), anyhow::Error> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn update_tokens(
        &mut self,
        access_token: String,
        refresh_token: String,
        user_id: Option<u64>,
    ) -> Result<(), anyhow::Error> {
        self.tidal.access_token = Some(access_token);
        self.tidal.refresh_token = Some(refresh_token);
        self.tidal.user_id = user_id;
        self.save()
    }
}

fn config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("tidal-subsonic");
    path.push("config.toml");
    path
}

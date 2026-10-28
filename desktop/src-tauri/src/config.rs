//! Persistent settings and paired devices, stored as JSON in the config dir.

use std::{fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{crypto, protocol};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    /// base64url device key.
    pub key: String,
    pub paired_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_id: String,
    pub server_name: String,
    pub port: u16,
    pub paused: bool,
    pub max_image_bytes: usize,
    /// Show a desktop notification when a clip arrives from a phone.
    pub notify_on_receive: bool,
    pub devices: Vec<PairedDevice>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_id: crypto::b64(&crypto::random_bytes::<16>()),
            server_name: gethostname::gethostname().to_string_lossy().into_owned(),
            port: protocol::DEFAULT_PORT,
            paused: false,
            max_image_bytes: 20 * 1024 * 1024,
            notify_on_receive: true,
            devices: Vec::new(),
        }
    }
}

pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            path: dir.join("config.json"),
        }
    }

    pub fn load(&self) -> Config {
        let config = fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        match config {
            Some(config) => config,
            None => {
                let config = Config::default();
                if let Err(err) = self.save(&config) {
                    log::warn!("could not write config: {err:#}");
                }
                config
            }
        }
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(config)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

//! Application state shared between the network server, the clipboard
//! watcher and the UI.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{
    clipboard::{self, LocalClip},
    config::{Config, ConfigStore, PairedDevice},
    crypto, pairing,
};

const HISTORY_LEN: usize = 50;
const PREVIEW_CHARS: usize = 400;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub id: String,
    pub mime: String,
    pub data: Arc<Vec<u8>>,
    pub sha256: String,
    pub ts: u64,
    /// `None` when the clip was copied on this computer.
    pub origin_device: Option<String>,
    pub origin_name: String,
}

impl Clip {
    fn is_supported_mime(mime: &str) -> bool {
        mime == clipboard::TEXT_MIME
    }
}

#[derive(Serialize)]
pub struct DeviceView {
    id: String,
    name: String,
    connected: bool,
    paired_at: u64,
}

#[derive(Serialize)]
pub struct ClipView {
    id: String,
    mime: String,
    preview: String,
    size: usize,
    origin: String,
    from_phone: bool,
    ts: u64,
}

#[derive(Serialize)]
pub struct Snapshot {
    server_name: String,
    port: u16,
    paused: bool,
    pair_uri: String,
    qr_svg: String,
    addresses: Vec<String>,
    devices: Vec<DeviceView>,
    history: Vec<ClipView>,
}

pub struct Core {
    store: ConfigStore,
    config: Mutex<Config>,
    pair_secret: Mutex<[u8; 32]>,
    connected: Mutex<HashMap<String, usize>>,
    history: Mutex<VecDeque<Clip>>,
    last_hash: Mutex<Option<String>>,
    outgoing: broadcast::Sender<Arc<Clip>>,
    on_change: Box<dyn Fn() + Send + Sync>,
}

impl Core {
    pub fn new(store: ConfigStore, on_change: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        let config = store.load();
        let (outgoing, _) = broadcast::channel(16);
        Arc::new(Self {
            store,
            config: Mutex::new(config),
            pair_secret: Mutex::new(crypto::random_bytes()),
            connected: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
            last_hash: Mutex::new(None),
            outgoing,
            on_change: Box::new(on_change),
        })
    }

    fn changed(&self) {
        (self.on_change)();
    }

    fn update_config(&self, f: impl FnOnce(&mut Config)) {
        let mut config = self.config.lock().unwrap();
        f(&mut config);
        if let Err(err) = self.store.save(&config) {
            log::error!("could not save config: {err:#}");
        }
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Clip>> {
        self.outgoing.subscribe()
    }

    // --- pairing ---------------------------------------------------------

    pub fn pair_secret(&self) -> [u8; 32] {
        *self.pair_secret.lock().unwrap()
    }

    pub fn rotate_pair_secret(&self) {
        *self.pair_secret.lock().unwrap() = crypto::random_bytes();
        self.changed();
    }

    pub fn device_key(&self, device_id: &str) -> Option<[u8; 32]> {
        let config = self.config.lock().unwrap();
        let device = config.devices.iter().find(|d| d.id == device_id)?;
        crypto::unb64(&device.key).ok()?.try_into().ok()
    }

    pub fn is_paired(&self, device_id: &str) -> bool {
        self.config
            .lock()
            .unwrap()
            .devices
            .iter()
            .any(|d| d.id == device_id)
    }

    pub fn add_device(&self, id: &str, name: &str, key: &[u8; 32]) {
        self.update_config(|config| {
            config.devices.retain(|d| d.id != id);
            config.devices.push(PairedDevice {
                id: id.into(),
                name: name.into(),
                key: crypto::b64(key),
                paired_at: now_ms(),
            });
        });
        self.changed();
    }

    pub fn update_device_name(&self, id: &str, name: &str) {
        let needs_update = self
            .config
            .lock()
            .unwrap()
            .devices
            .iter()
            .any(|d| d.id == id && d.name != name);
        if needs_update {
            self.update_config(|config| {
                if let Some(device) = config.devices.iter_mut().find(|d| d.id == id) {
                    device.name = name.into();
                }
            });
        }
    }

    pub fn remove_device(&self, id: &str) {
        self.update_config(|config| config.devices.retain(|d| d.id != id));
        self.changed();
    }

    pub fn set_connected(&self, device_id: &str, connected: bool) {
        {
            let mut map = self.connected.lock().unwrap();
            let count = map.entry(device_id.to_string()).or_default();
            if connected {
                *count += 1;
            } else {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    map.remove(device_id);
                }
            }
        }
        self.changed();
    }

    pub fn set_paused(&self, paused: bool) {
        self.update_config(|config| config.paused = paused);
        self.changed();
    }

    pub fn paused(&self) -> bool {
        self.config.lock().unwrap().paused
    }

    // --- clips -----------------------------------------------------------

    /// Records a clip as current and returns `false` if it was already current.
    fn mark_current(&self, sha256: &str) -> bool {
        let mut last = self.last_hash.lock().unwrap();
        if last.as_deref() == Some(sha256) {
            return false;
        }
        *last = Some(sha256.to_string());
        true
    }

    fn push_history(&self, clip: Clip) {
        let mut history = self.history.lock().unwrap();
        history.retain(|c| c.sha256 != clip.sha256);
        history.push_front(clip);
        history.truncate(HISTORY_LEN);
    }

    /// Called by the clipboard watcher when something was copied on this computer.
    pub fn local_clip_changed(&self, local: LocalClip, initial: bool) {
        let sha256 = crypto::sha256_hex(&local.data);
        if !self.mark_current(&sha256) || initial {
            return;
        }
        let clip = Clip {
            id: uuid::Uuid::new_v4().to_string(),
            mime: local.mime,
            data: Arc::new(local.data),
            sha256,
            ts: now_ms(),
            origin_device: None,
            origin_name: "This computer".into(),
        };
        self.push_history(clip.clone());
        if !self.paused() {
            let _ = self.outgoing.send(Arc::new(clip));
        }
        self.changed();
    }

    /// Called by a session when a phone sent a clip.
    pub fn remote_clip(
        &self,
        device_id: &str,
        device_name: &str,
        id: String,
        mime: String,
        sha256: String,
        body: &[u8],
    ) -> Result<()> {
        if !Clip::is_supported_mime(&mime) {
            bail!("unsupported mime type {mime}");
        }
        if crypto::sha256_hex(body) != sha256 {
            bail!("clip hash mismatch");
        }
        if self.paused() {
            return Ok(());
        }
        let clip = Clip {
            id,
            mime,
            data: Arc::new(body.to_vec()),
            sha256,
            ts: now_ms(),
            origin_device: Some(device_id.to_string()),
            origin_name: device_name.to_string(),
        };
        if self.mark_current(&clip.sha256) {
            clipboard::set(&LocalClip {
                mime: clip.mime.clone(),
                data: clip.data.to_vec(),
            })?;
            // Forward to any other connected phones.
            let _ = self.outgoing.send(Arc::new(clip.clone()));
        }
        self.push_history(clip);
        self.changed();
        Ok(())
    }

    /// Puts a history entry back on the clipboard. The clipboard watcher then
    /// picks it up and syncs it like any other copy.
    pub fn recopy(&self, id: &str) -> Result<()> {
        let clip = self
            .history
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned();
        let Some(clip) = clip else {
            bail!("clip not found");
        };
        // Force the watcher to treat it as new so phones receive it again.
        *self.last_hash.lock().unwrap() = None;
        clipboard::set(&LocalClip {
            mime: clip.mime,
            data: clip.data.to_vec(),
        })
    }

    pub fn clear_history(&self) {
        self.history.lock().unwrap().clear();
        self.changed();
    }

    pub fn snapshot(&self) -> Snapshot {
        let config = self.config();
        let addresses = pairing::lan_addresses();
        let pair_uri = pairing::pair_uri(&config, &self.pair_secret(), &addresses);
        #[cfg(debug_assertions)]
        log::debug!("pairing uri: {pair_uri}");
        let connected = self.connected.lock().unwrap();
        let devices = config
            .devices
            .iter()
            .map(|d| DeviceView {
                id: d.id.clone(),
                name: d.name.clone(),
                connected: connected.contains_key(&d.id),
                paired_at: d.paired_at,
            })
            .collect();
        let history = self
            .history
            .lock()
            .unwrap()
            .iter()
            .map(|c| ClipView {
                id: c.id.clone(),
                mime: c.mime.clone(),
                preview: if c.mime == clipboard::TEXT_MIME {
                    String::from_utf8_lossy(&c.data)
                        .chars()
                        .take(PREVIEW_CHARS)
                        .collect()
                } else {
                    String::new()
                },
                size: c.data.len(),
                origin: c.origin_name.clone(),
                from_phone: c.origin_device.is_some(),
                ts: c.ts,
            })
            .collect();
        Snapshot {
            qr_svg: pairing::qr_svg(&pair_uri),
            pair_uri,
            server_name: config.server_name,
            port: config.port,
            paused: config.paused,
            addresses: addresses.iter().map(ToString::to_string).collect(),
            devices,
            history,
        }
    }
}
